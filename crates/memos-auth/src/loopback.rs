//! The one-shot web server that catches the redirect.
//!
//! ## Why a loopback address and not a custom scheme
//!
//! A native app has two ways to receive an OAuth redirect: register a URI
//! scheme (`memos://callback`) or listen on `http://127.0.0.1:<port>`. RFC 8252
//! permits both and this picks loopback, for reasons that matter more here than
//! usual:
//!
//! - A scheme handler is registered by the *installer*. There is no installer
//!   yet, so a scheme would work only for developers and silently fail for the
//!   first real user.
//! - Scheme registration is per-platform and, on Windows, per-user registry
//!   writes. Loopback is the same few lines of code on Windows and macOS, which
//!   is the port that is actually coming.
//! - A scheme handler launches a *second* instance of the app to deliver the
//!   URL, which then has to hand it to the running one. This app already has a
//!   single-instance guard; threading a callback through it is a second
//!   mechanism to get wrong.
//!
//! The cost is that any local process can talk to this port, which is exactly
//! what PKCE and the `state` parameter are there to make harmless.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::{AuthError, AuthResult};

/// Listens on an ephemeral loopback port for exactly one callback.
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

/// What the browser handed back.
#[derive(Debug, Clone, PartialEq)]
pub struct Callback {
    pub code: String,
    pub state: String,
}

impl Loopback {
    /// Bind to a port the operating system chooses.
    ///
    /// Port 0 rather than a fixed one: a fixed port is unavailable when
    /// something else has it, and worse, is squattable by any other process on
    /// the machine that knows which port this app uses.
    pub fn bind() -> AuthResult<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .map_err(|e| AuthError::Loopback(format!("could not open a local port: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| AuthError::Loopback(e.to_string()))?
            .port();
        Ok(Self { listener, port })
    }

    /// The address to hand the authorisation server as `redirect_uri`.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }

    /// Wait for the browser, up to `timeout`.
    ///
    /// The timeout is generous and it is not a formality: between here and the
    /// callback sits a human choosing an account, possibly creating one, and
    /// possibly reading a consent screen. What it protects against is the tab
    /// that gets closed instead — without it this thread would wait forever on
    /// something that is never coming.
    ///
    /// Connections that are not the callback are answered and ignored rather
    /// than ending the wait. A browser will happily ask for `/favicon.ico` on
    /// this port, and treating that as the answer would abort every sign-in.
    pub fn wait(&self, expected_state: &str, timeout: Duration) -> AuthResult<Callback> {
        let deadline = Instant::now() + timeout;
        self.listener
            .set_nonblocking(false)
            .map_err(|e| AuthError::Loopback(e.to_string()))?;

        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(AuthError::TimedOut);
            }
            // Applied per accept so a stream of junk requests cannot extend the
            // wait indefinitely — the deadline above is recomputed each time.
            self.listener
                .set_nonblocking(true)
                .map_err(|e| AuthError::Loopback(e.to_string()))?;
            let accepted = self.listener.accept();
            self.listener
                .set_nonblocking(false)
                .map_err(|e| AuthError::Loopback(e.to_string()))?;

            let mut stream = match accepted {
                Ok((s, _)) => s,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(60));
                    continue;
                }
                Err(e) => return Err(AuthError::Loopback(e.to_string())),
            };

            let Some(target) = read_request_target(&mut stream) else {
                respond(&mut stream, PLAIN, "bad request");
                continue;
            };

            match parse_callback(&target) {
                Ok(cb) if cb.state == expected_state => {
                    respond(&mut stream, HTML, DONE);
                    return Ok(cb);
                }
                // A callback for some other attempt, or one nobody asked for.
                // Answered politely and otherwise ignored: this is the case the
                // `state` parameter exists to catch, and treating it as an
                // error would let anyone with a browser cancel a sign-in.
                Ok(_) => {
                    tracing::warn!("discarded a callback whose state did not match");
                    respond(&mut stream, HTML, WRONG);
                }
                Err(AuthError::Denied(reason)) => {
                    respond(&mut stream, HTML, DENIED);
                    return Err(AuthError::Denied(reason));
                }
                Err(_) => respond(&mut stream, PLAIN, "not found"),
            }
        }
    }
}

/// The request target from the first line: `GET /callback?... HTTP/1.1`.
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    // Only the request line is read. The headers and body are of no interest,
    // and reading to EOF on a keep-alive connection would block until timeout.
    BufReader::new(stream).read_line(&mut line).ok()?;
    line.split_whitespace().nth(1).map(str::to_string)
}

/// Pull `code` and `state` out of the callback's query string.
pub(crate) fn parse_callback(target: &str) -> AuthResult<Callback> {
    // Parsed against a base because the request target is a path, not an
    // absolute URL. The host here is a placeholder and is never used.
    let url = url::Url::parse("http://127.0.0.1")
        .and_then(|base| base.join(target))
        .map_err(|e| AuthError::Loopback(e.to_string()))?;

    if url.path() != "/callback" {
        return Err(AuthError::Loopback(format!("unexpected path {}", url.path())));
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            "error_description" => description = Some(v.into_owned()),
            _ => {}
        }
    }

    // The provider saying no is a normal outcome — the user pressed cancel —
    // and it has to be distinguishable from a malformed request, because one
    // means "stop asking" and the other means "something is broken".
    if let Some(error) = error {
        return Err(AuthError::Denied(description.unwrap_or(error)));
    }

    match (code, state) {
        (Some(code), Some(state)) => Ok(Callback { code, state }),
        _ => Err(AuthError::Loopback("callback had no code".into())),
    }
}

const HTML: &str = "text/html; charset=utf-8";
const PLAIN: &str = "text/plain; charset=utf-8";

fn respond(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// The three pages this ever serves.
///
/// Deliberately plain, and deliberately not styled to look like the app. They
/// are served by a local port over plain HTTP, and a page that looked like part
/// of the product would be teaching people to trust exactly the thing they
/// should not: a convincing local page that appeared while they were signing in
/// to something.
static DONE: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title><body style=\"font-family:system-ui;margin:14vh auto;max-width:30rem;text-align:center\"><h2 style=\"font-weight:600\">Signed in</h2><p style=\"color:#555\">You can close this tab and go back to Memory OS.</p>";
static WRONG: &str = "<!doctype html><meta charset=utf-8><title>Unexpected response</title><body style=\"font-family:system-ui;margin:14vh auto;max-width:30rem;text-align:center\"><h2 style=\"font-weight:600\">Unexpected response</h2><p style=\"color:#555\">This did not match the sign-in that was started. Nothing has changed. Try again from the app.</p>";
static DENIED: &str = "<!doctype html><meta charset=utf-8><title>Not signed in</title><body style=\"font-family:system-ui;margin:14vh auto;max-width:30rem;text-align:center\"><h2 style=\"font-weight:600\">Not signed in</h2><p style=\"color:#555\">You can close this tab. Memory OS works exactly as before.</p>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_good_callback_yields_its_code_and_state() {
        let cb = parse_callback("/callback?code=abc123&state=xyz").unwrap();
        assert_eq!(cb.code, "abc123");
        assert_eq!(cb.state, "xyz");
    }

    #[test]
    fn percent_encoding_is_decoded() {
        let cb = parse_callback("/callback?code=a%2Fb%2Bc&state=s").unwrap();
        assert_eq!(cb.code, "a/b+c", "a code is opaque and may contain anything");
    }

    /// The user pressing cancel is not a bug, and it must not read like one.
    #[test]
    fn a_refusal_is_its_own_kind_of_answer() {
        let e = parse_callback("/callback?error=access_denied&state=s").unwrap_err();
        assert!(matches!(e, AuthError::Denied(_)), "{e:?}");
    }

    #[test]
    fn a_refusal_prefers_the_readable_description() {
        let e = parse_callback(
            "/callback?error=access_denied&error_description=You%20cancelled&state=s",
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "sign-in was not completed: You cancelled");
    }

    /// The browser asks for this on its own, and answering it must not be
    /// mistaken for the user having finished.
    #[test]
    fn a_favicon_request_is_not_a_callback() {
        assert!(parse_callback("/favicon.ico").is_err());
    }

    #[test]
    fn a_callback_with_no_code_is_rejected() {
        assert!(parse_callback("/callback?state=s").is_err());
    }

    #[test]
    fn the_redirect_uri_names_the_port_that_was_opened() {
        let lo = Loopback::bind().unwrap();
        assert_eq!(lo.redirect_uri(), format!("http://127.0.0.1:{}/callback", lo.port));
        assert!(lo.port > 0, "the OS must have chosen a real port");
    }

    /// Two sign-in attempts must never collide on a port.
    #[test]
    fn every_bind_gets_its_own_port() {
        let (a, b) = (Loopback::bind().unwrap(), Loopback::bind().unwrap());
        assert_ne!(a.port, b.port);
    }

    #[test]
    fn waiting_gives_up_rather_than_hanging_forever() {
        let lo = Loopback::bind().unwrap();
        let e = lo.wait("state", Duration::from_millis(150)).unwrap_err();
        assert!(matches!(e, AuthError::TimedOut), "{e:?}");
    }
}
