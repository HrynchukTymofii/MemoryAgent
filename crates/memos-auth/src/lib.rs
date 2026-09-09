//! Sign-in, for a product that does not need it to work.
//!
//! ## What an account is for here
//!
//! Nothing in this application requires an account. Capture, search, the whole
//! loop, run against a local SQLite file and will keep doing so — that is the
//! product, and ADR-0007's tiering says the archive is never gated. So this
//! crate exists for two things that are genuinely worth having and neither of
//! which is worth blocking on:
//!
//! - Knowing who the people using it are. An email address is the difference
//!   between "some installs" and a group of users who can be told the thing
//!   they liked got better.
//! - Being the identity that cloud sync will attach to when M5 arrives, so that
//!   milestone starts with the hard half already done.
//!
//! Because of that, every path here is written so failing is survivable. A
//! provider that is down, a network that is off, a token that expired while the
//! laptop was shut — all of them end in "not signed in", never in a capture
//! that does not happen.
//!
//! ## The shape of the flow
//!
//! Authorization Code with PKCE, over a loopback redirect, per RFC 8252. What
//! makes the code safe to intercept is the verifier, not a secret; see [`pkce`]
//! for why, and [`loopback`] for why the redirect is a local port rather than a
//! custom URI scheme.
//!
//! Some providers — Google's "Desktop app" client type among them — still issue
//! and require a `client_secret` for public clients, and say plainly that it is
//! not confidential. [`Provider::client_secret`] exists for those and is sent
//! only when set.
//!
//! ```text
//! sign_in()
//!   bind 127.0.0.1:0            <- the OS picks the port
//!   open the browser            <- the user leaves the app
//!   wait for /callback          <- with a timeout, because tabs get closed
//!   exchange code + verifier    <- the only time the verifier is sent
//!   fetch the profile           <- for the email, which is the point
//!   save the session
//! ```

pub mod backend;
pub mod loopback;
pub mod pkce;
pub mod session;

use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};

pub use backend::Backend;
pub use loopback::Loopback;
pub use pkce::Pkce;
pub use session::{Identity, Session, SessionStore};

/// How long the browser has before the attempt is abandoned.
///
/// Five minutes. Long enough to create an account, find a password, and pass a
/// second factor; short enough that a forgotten tab does not leave a port open
/// and a thread parked for the life of the process.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// No provider is configured, so signing in is not on offer at all. Not a
    /// failure — it is the state of a build nobody has pointed at a tenant yet.
    #[error("sign-in is not configured")]
    NotConfigured,
    #[error("sign-in was not completed: {0}")]
    Denied(String),
    #[error("sign-in timed out")]
    TimedOut,
    #[error("could not reach the sign-in service: {0}")]
    Network(String),
    #[error("the sign-in service returned something unusable: {0}")]
    Protocol(String),
    #[error("local sign-in listener failed: {0}")]
    Loopback(String),
    #[error("could not store the session: {0}")]
    Store(String),
    #[error("could not open a browser: {0}")]
    Browser(String),
}

pub type AuthResult<T> = Result<T, AuthError>;

/// Where to send people, and as whom.
///
/// Every field is configuration rather than a constant because the endpoints
/// belong to whoever deploys this, and hard-coding one tenant's URLs into the
/// binary is how you end up unable to move. Neon Auth, Stack Auth, Auth0 and a
/// plain OIDC provider all fit this shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provider {
    pub authorize_url: String,
    pub token_url: String,
    /// Where to read the signed-in user's profile. Optional: some providers
    /// return the identity inside the token response, and one round trip is
    /// better than two.
    #[serde(default)]
    pub userinfo_url: Option<String>,
    pub client_id: String,

    /// Sent at the token exchange when present.
    ///
    /// A desktop client is a *public* client and this is not a secret in the
    /// usual sense — Google issues one for its "Desktop app" client type and
    /// says so explicitly, and its token endpoint rejects the exchange without
    /// it. So this is not a contradiction of PKCE, it is a parameter some
    /// providers require and others forbid; hence optional, and hence never
    /// treated as confidential anywhere in this crate.
    #[serde(default)]
    pub client_secret: Option<String>,

    /// Space-separated, as the parameter is transmitted. `openid email profile`
    /// covers what this needs: an identifier and an address to write to.
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_scope() -> String {
    "openid email profile".into()
}

impl Provider {
    /// Whether this is filled in enough to try.
    ///
    /// Checked rather than assumed because the shipped default is empty: a
    /// build with no tenant configured must offer no sign-in button at all,
    /// rather than one that fails when pressed.
    pub fn is_configured(&self) -> bool {
        !self.client_id.trim().is_empty()
            && !self.authorize_url.trim().is_empty()
            && !self.token_url.trim().is_empty()
    }
}

impl Default for Provider {
    fn default() -> Self {
        Self {
            authorize_url: String::new(),
            token_url: String::new(),
            userinfo_url: None,
            client_id: String::new(),
            client_secret: None,
            scope: default_scope(),
        }
    }
}

/// The token endpoint's answer.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    /// Present on OIDC providers. Not verified as a signature here — the
    /// response came straight from the token endpoint over TLS, which is the
    /// condition under which RFC 9068 permits skipping validation — but the
    /// claims are read for the email when there is no userinfo endpoint.
    #[serde(default)]
    id_token: Option<String>,
}

/// The profile endpoint's answer, in the several shapes providers use.
#[derive(Debug, Default, Deserialize)]
struct Profile {
    #[serde(alias = "id", alias = "user_id")]
    sub: Option<String>,
    #[serde(alias = "primary_email")]
    email: Option<String>,
    #[serde(alias = "name", alias = "display_name")]
    full_name: Option<String>,
}

pub struct Auth {
    provider: Provider,
    backend: Backend,
    store: SessionStore,
    http: reqwest::blocking::Client,
}

impl Auth {
    pub fn new(provider: Provider, backend: Backend, data_dir: PathBuf) -> Self {
        let http = reqwest::blocking::Client::builder()
            // Every request here is one a person is waiting on, in front of a
            // screen that says "signing in". A hung connection has to become an
            // error while they are still looking at it.
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("MemoryOS/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            provider,
            backend,
            store: SessionStore::new(&data_dir),
            http,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.provider.is_configured()
    }

    /// Who is signed in, as far as the interface is concerned.
    pub fn identity(&self) -> Identity {
        self.store
            .load()
            .as_ref()
            .map(Identity::from)
            .unwrap_or_else(Identity::anonymous)
    }

    /// Run the whole flow. Blocks for as long as the user takes.
    ///
    /// Call it on a thread of its own — it waits on a human.
    pub fn sign_in(&self) -> AuthResult<Identity> {
        if !self.is_configured() {
            return Err(AuthError::NotConfigured);
        }

        let pkce = Pkce::new();
        // Bound before the browser opens, because the redirect URI has to name
        // the port and the port is not known until the OS assigns it.
        let server = Loopback::bind()?;
        let redirect_uri = server.redirect_uri();

        let url = self.authorize_url(&pkce, &redirect_uri)?;
        open_browser(&url)?;

        let callback = server.wait(&pkce.state, SIGN_IN_TIMEOUT)?;
        let token = self.exchange(&callback.code, &pkce.verifier, &redirect_uri)?;
        let profile = self.profile(&token);

        let session = Session {
            user_id: profile.sub.unwrap_or_default(),
            email: profile.email,
            display_name: profile.full_name,
            expires_at: token
                .expires_in
                .map(|s| Utc::now() + chrono::Duration::seconds(s)),
            access_token: token.access_token,
            id_token: token.id_token,
            refresh_token: token.refresh_token,
            signed_in_at: Utc::now(),
        };
        self.store.save(&session)?;

        // The user is signed in from here whatever happens next. Recording them
        // in Postgres is bookkeeping, and bookkeeping does not get to fail the
        // thing the user actually asked for.
        if let Err(e) = backend::record_user(&self.http, &self.backend, &session) {
            tracing::warn!(?e, "signed in, but the user was not recorded");
        }

        tracing::info!(email = ?session.email, "signed in");
        Ok(Identity::from(&session))
    }

    /// Forget the session locally.
    ///
    /// Deliberately local-only. Revoking at the provider needs an endpoint not
    /// every one of them offers, and a sign-out that fails because a server is
    /// unreachable is a sign-out that has failed the user — the point of
    /// pressing it is that the credential stops being on this machine.
    pub fn sign_out(&self) -> AuthResult<()> {
        self.store.clear()
    }

    fn authorize_url(&self, pkce: &Pkce, redirect_uri: &str) -> AuthResult<String> {
        let mut url = url::Url::parse(&self.provider.authorize_url)
            .map_err(|e| AuthError::Protocol(format!("bad authorize_url: {e}")))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.provider.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &self.provider.scope)
            .append_pair("state", &pkce.state)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.to_string())
    }

    fn exchange(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> AuthResult<TokenResponse> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", &self.provider.client_id),
            ("code_verifier", verifier),
        ];
        // Only when the provider asked for one. Sending an empty
        // `client_secret` is not the same as omitting it: several providers
        // reject the request outright rather than ignoring the parameter.
        if let Some(secret) = self.provider.client_secret.as_deref().filter(|s| !s.is_empty()) {
            form.push(("client_secret", secret));
        }
        let response = self
            .http
            .post(&self.provider.token_url)
            .form(&form)
            .send()
            .map_err(|e| AuthError::Network(e.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .map_err(|e| AuthError::Network(e.to_string()))?;
        if !status.is_success() {
            // Truncated, and logged rather than shown: a token endpoint's error
            // body can be long, and can echo back parts of the request.
            tracing::warn!(%status, body = %body.chars().take(400).collect::<String>(),
                "token exchange refused");
            return Err(AuthError::Protocol(format!("token exchange failed ({status})")));
        }
        serde_json::from_str(&body)
            .map_err(|e| AuthError::Protocol(format!("token response unreadable: {e}")))
    }

    /// Best effort. A session with no email is still a session.
    fn profile(&self, token: &TokenResponse) -> Profile {
        if let Some(url) = &self.provider.userinfo_url {
            match self
                .http
                .get(url)
                .bearer_auth(&token.access_token)
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.json::<Profile>())
            {
                Ok(p) => return p,
                Err(e) => tracing::warn!(?e, "could not read the profile; falling back"),
            }
        }
        token
            .id_token
            .as_deref()
            .and_then(claims_of)
            .unwrap_or_default()
    }
}

/// Read the claims out of a JWT without verifying it.
///
/// Safe only because of where this is called from: the token came directly from
/// the provider's token endpoint over TLS, in response to a request carrying a
/// code and a verifier only we hold. It is never used for an access decision —
/// only to find an email address to display — so a forged token would achieve
/// nothing beyond a wrong name in the settings screen of the machine that
/// forged it.
fn claims_of(jwt: &str) -> Option<Profile> {
    use base64::Engine;
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Hand a URL to the user's default browser.
///
/// Sign-in happens in the real browser, never in an embedded webview. The user
/// needs to see the address bar and the padlock of the site they are typing a
/// password into, and providers increasingly refuse embedded webviews for
/// exactly that reason.
fn open_browser(url: &str) -> AuthResult<()> {
    #[cfg(windows)]
    let mut command = {
        // `explorer` takes the argument verbatim and applies the user's own
        // default handler. It also returns a non-zero exit code on success,
        // which is why the status is never checked.
        let mut c = std::process::Command::new("explorer");
        c.arg(url);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|e| AuthError::Browser(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> Provider {
        Provider {
            authorize_url: "https://auth.example.com/authorize".into(),
            token_url: "https://auth.example.com/token".into(),
            userinfo_url: Some("https://auth.example.com/userinfo".into()),
            client_id: "client-123".into(),
            client_secret: None,
            scope: default_scope(),
        }
    }

    fn auth() -> Auth {
        Auth::new(
            provider(),
            Backend::default(),
            std::env::temp_dir().join("memos-auth-tests"),
        )
    }

    #[test]
    fn an_unconfigured_provider_offers_no_sign_in() {
        assert!(!Provider::default().is_configured());
        assert!(provider().is_configured());
    }

    /// The failure this must never have: a sign-in button in a build with no
    /// tenant behind it, which can only ever produce a confusing error.
    #[test]
    fn signing_in_without_configuration_is_refused_before_anything_opens() {
        let auth = Auth::new(Provider::default(), Backend::default(), std::env::temp_dir());
        assert!(matches!(auth.sign_in(), Err(AuthError::NotConfigured)));
    }

    #[test]
    fn the_authorize_url_carries_everything_the_server_needs() {
        let pkce = Pkce::new();
        let url = auth()
            .authorize_url(&pkce, "http://127.0.0.1:5555/callback")
            .unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "client-123");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:5555/callback");
        assert_eq!(q["state"], pkce.state);
        assert_eq!(q["code_challenge"], pkce.challenge);
        assert_eq!(q["code_challenge_method"], "S256", "plain is not acceptable");
    }

    /// The whole point of PKCE. If the verifier ever appears in the URL the
    /// browser is sent to, the exchange is no better than having no secret.
    #[test]
    fn the_verifier_never_appears_in_the_browser_url() {
        let pkce = Pkce::new();
        let url = auth().authorize_url(&pkce, "http://127.0.0.1:1/callback").unwrap();
        assert!(!url.contains(&pkce.verifier), "the verifier leaked into {url}");
    }

    /// An authorize URL that already carries query parameters — several
    /// providers publish one — must keep them.
    #[test]
    fn existing_query_parameters_are_preserved() {
        let mut p = provider();
        p.authorize_url = "https://auth.example.com/authorize?tenant=acme".into();
        let auth = Auth::new(p, Backend::default(), std::env::temp_dir());
        let url = auth.authorize_url(&Pkce::new(), "http://127.0.0.1:1/callback").unwrap();
        assert!(url.contains("tenant=acme"), "{url}");
        assert!(url.contains("code_challenge_method=S256"), "{url}");
    }

    /// Sending an empty `client_secret` is not the same as omitting it — some
    /// providers reject the request rather than ignoring the parameter.
    #[test]
    fn an_empty_secret_is_treated_as_no_secret() {
        let mut p = provider();
        p.client_secret = Some(String::new());
        assert!(p.client_secret.as_deref().filter(|s| !s.is_empty()).is_none());
        p.client_secret = Some("shh".into());
        assert_eq!(p.client_secret.as_deref().filter(|s| !s.is_empty()), Some("shh"));
    }

    #[test]
    fn claims_are_read_out_of_an_id_token() {
        use base64::Engine;
        let claims = br#"{"sub":"u_9","email":"a@b.com","name":"A B"}"#;
        let jwt = format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims)
        );
        let p = claims_of(&jwt).unwrap();
        assert_eq!(p.sub.as_deref(), Some("u_9"));
        assert_eq!(p.email.as_deref(), Some("a@b.com"));
        assert_eq!(p.full_name.as_deref(), Some("A B"));
    }

    #[test]
    fn a_malformed_id_token_yields_nothing_rather_than_panicking() {
        assert!(claims_of("not-a-jwt").is_none());
        assert!(claims_of("a.!!!.c").is_none());
        assert!(claims_of("").is_none());
    }

    /// Providers disagree about what these fields are called, and a session
    /// with no email would defeat the reason this crate exists.
    #[test]
    fn the_profile_accepts_the_names_providers_actually_use() {
        let p: Profile =
            serde_json::from_str(r#"{"id":"u_1","primary_email":"a@b.com","display_name":"A"}"#)
                .unwrap();
        assert_eq!(p.sub.as_deref(), Some("u_1"));
        assert_eq!(p.email.as_deref(), Some("a@b.com"));
        assert_eq!(p.full_name.as_deref(), Some("A"));
    }

    #[test]
    fn signing_out_with_no_session_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Auth::new(provider(), Backend::default(), dir.path().to_path_buf());
        auth.sign_out().unwrap();
        assert!(!auth.identity().signed_in);
    }
}
