//! Talking to our own API.
//!
//! ## Why there is a service in the middle
//!
//! A desktop application cannot hold a database password. It ships to everyone,
//! it is readable by everyone, and a Postgres URL inside it grants whoever
//! extracts it every row every user has ever written. No configuration makes
//! that safe.
//!
//! So the app holds no connection string and speaks no SQL. It sends the
//! identity token it got from signing in to our API, which verifies it against
//! the provider's public keys, records the user, and returns a session token of
//! our own. Postgres is reachable only from that service.
//!
//! ```text
//! desktop --id_token--> API --verifies--> Google
//!                        |
//!                        +--SQL--> Postgres
//!                        |
//!         <--session token--
//! ```
//!
//! The session token replaces Google's, deliberately. Google's identity token
//! expires in an hour and renewing it means going back to Google; ours is
//! issued by the service that will answer every later request anyway, lasts as
//! long as we choose, and is revoked by rotating one secret.

use serde::{Deserialize, Serialize};

use crate::{AuthError, AuthResult, Session};

/// Where our API lives. Empty means there is none, which is a supported state:
/// the app signs in, keeps the session locally, and records nothing anywhere.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Backend {
    /// Base URL, without a trailing slash.
    #[serde(default)]
    pub api_url: String,
}

impl Backend {
    pub fn is_configured(&self) -> bool {
        !self.api_url.trim().is_empty()
    }
}

/// What we send: the assertion the provider made about this person.
#[derive(Debug, Serialize)]
struct SignInBody<'a> {
    id_token: &'a str,
}

/// What comes back: our own session, and the account as the server has it.
#[derive(Debug, Deserialize)]
pub struct Registered {
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub account: Account,
}

#[derive(Debug, Deserialize)]
pub struct Account {
    pub id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
}

/// Exchange the provider's identity token for a session with our API.
///
/// Deliberately not fatal to signing in. The user is signed in locally by the
/// time this runs, and losing that because a server is unreachable would trade
/// the thing they asked for against the thing they did not. What is lost when
/// this fails is sync and the record of them existing, and the next sign-in
/// re-establishes both.
pub fn register(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    session: &Session,
) -> AuthResult<Registered> {
    if !backend.is_configured() {
        return Err(AuthError::NotConfigured);
    }
    let Some(id_token) = session.id_token.as_deref() else {
        // An access token cannot be verified by anyone but its issuer. Only the
        // identity token carries claims a third party can check.
        return Err(AuthError::Protocol("no identity token to present".into()));
    };

    let url = format!("{}/v1/auth/google", backend.api_url.trim_end_matches('/'));
    let response = http
        .post(&url)
        .json(&SignInBody { id_token })
        .send()
        .map_err(|e| AuthError::Network(e.to_string()))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        tracing::warn!(%status, body = %body.chars().take(300).collect::<String>(),
            "the API refused the sign-in");
        return Err(AuthError::Protocol(explain(status.as_u16())));
    }
    serde_json::from_str(&body)
        .map_err(|e| AuthError::Protocol(format!("unreadable response from the API: {e}")))
}

/// Ask the API to send a one-time code to an address.
pub fn email_start(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    email: &str,
) -> AuthResult<()> {
    if !backend.is_configured() {
        return Err(AuthError::NotConfigured);
    }
    let url = format!("{}/v1/auth/email/start", backend.api_url.trim_end_matches('/'));
    let response = http
        .post(&url)
        .json(&serde_json::json!({ "email": email }))
        .send()
        .map_err(|e| AuthError::Network(e.to_string()))?;

    match response.status().as_u16() {
        200..=299 => Ok(()),
        400 => Err(AuthError::Protocol("that does not look like an email address".into())),
        501 => Err(AuthError::NotConfigured),
        other => Err(AuthError::Protocol(explain(other))),
    }
}

/// Exchange a code for a session.
pub fn email_verify(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    email: &str,
    code: &str,
) -> AuthResult<Registered> {
    if !backend.is_configured() {
        return Err(AuthError::NotConfigured);
    }
    let url = format!("{}/v1/auth/email/verify", backend.api_url.trim_end_matches('/'));
    let response = http
        .post(&url)
        .json(&serde_json::json!({ "email": email, "code": code }))
        .send()
        .map_err(|e| AuthError::Network(e.to_string()))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        // The server deliberately does not say which way it was wrong — no
        // code, wrong code, or too many tries — so neither does this.
        return Err(match status.as_u16() {
            401 => AuthError::Denied("that code is not valid".into()),
            other => AuthError::Protocol(explain(other)),
        });
    }
    serde_json::from_str(&body)
        .map_err(|e| AuthError::Protocol(format!("unreadable response from the API: {e}")))
}

// ----------------------------------------------------------------- referrals

/// One person the user brought in, as the API is willing to describe them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferralRow {
    /// Masked by the server: `t…@gmail.com`. The referrer is owed a count and a
    /// status, not somebody else's address.
    pub who: String,
    /// `pending` until the referee has used the app enough, then `qualified`.
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub qualified_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Everything the referral screen draws.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferralStatus {
    pub code: String,
    pub link: String,
    pub qualify_words: u32,
    pub months_per_referral: u32,
    pub referrals: Vec<ReferralRow>,
    pub months_earned: u32,
    pub pro_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this account may still be referred by somebody else.
    pub can_apply: bool,
    pub applied_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    /// True only on the call that actually paid out, never on the ones after.
    pub qualified: bool,
    pub pro_until: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invited {
    pub sent: Vec<String>,
    pub failed: Vec<String>,
}

/// The caller's referral code, invites and rewards.
pub fn referral_status(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    api_token: &str,
) -> AuthResult<ReferralStatus> {
    get_json(http, backend, api_token, "/v1/referrals/me")
}

/// Be referred by somebody.
///
/// The refusals here are worth distinguishing, unlike the sign-in ones: every
/// single one is something the user can act on — a code that does not exist, a
/// code that is their own, an account already referred, an account too old.
/// Telling them which is not telling an attacker anything they did not type.
pub fn apply_referral(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    api_token: &str,
    code: &str,
) -> AuthResult<ReferralStatus> {
    post_json(
        http,
        backend,
        api_token,
        "/v1/referrals/apply",
        &serde_json::json!({ "code": code }),
    )
}

/// Mail an invite to each address.
pub fn send_invites(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    api_token: &str,
    emails: &[String],
) -> AuthResult<Invited> {
    post_json(
        http,
        backend,
        api_token,
        "/v1/referrals/invite",
        &serde_json::json!({ "emails": emails }),
    )
}

/// Report lifetime words, which is what pays a pending referral out.
///
/// Sent from the client because the server has no other way to know: transcripts
/// never leave the machine, and shipping them somewhere so a month of Pro can be
/// awarded honestly would be a far worse trade than the one this makes. The
/// constraints that matter — one referral per account, one payout per side —
/// are in the database and are not client-side at all.
pub fn report_progress(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    api_token: &str,
    words: u64,
) -> AuthResult<Progress> {
    post_json(
        http,
        backend,
        api_token,
        "/v1/referrals/progress",
        &serde_json::json!({ "words": words }),
    )
}

fn get_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    api_token: &str,
    path: &str,
) -> AuthResult<T> {
    if !backend.is_configured() {
        return Err(AuthError::NotConfigured);
    }
    let url = format!("{}{path}", backend.api_url.trim_end_matches('/'));
    let response = http
        .get(&url)
        .bearer_auth(api_token)
        .send()
        .map_err(|e| AuthError::Network(e.to_string()))?;
    read(response)
}

fn post_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    api_token: &str,
    path: &str,
    body: &serde_json::Value,
) -> AuthResult<T> {
    if !backend.is_configured() {
        return Err(AuthError::NotConfigured);
    }
    let url = format!("{}{path}", backend.api_url.trim_end_matches('/'));
    let response = http
        .post(&url)
        .bearer_auth(api_token)
        .json(body)
        .send()
        .map_err(|e| AuthError::Network(e.to_string()))?;
    read(response)
}

/// Turn a response into a value, or into a message worth showing.
///
/// The API's own `detail` is preferred over anything invented here, because it
/// is the only party that knows *which* rule refused: "that is your own code"
/// and "this account has already been referred" are both 400-shaped and mean
/// completely different things to the person reading them.
fn read<T: serde::de::DeserializeOwned>(response: reqwest::blocking::Response) -> AuthResult<T> {
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("detail").and_then(|d| d.as_str().map(str::to_string)));
        return Err(match status.as_u16() {
            401 => AuthError::Denied("sign in again to do that".into()),
            501 => AuthError::NotConfigured,
            other => AuthError::Protocol(detail.unwrap_or_else(|| explain(other))),
        });
    }
    serde_json::from_str(&body)
        .map_err(|e| AuthError::Protocol(format!("unreadable response from the API: {e}")))
}

/// Turn a refusal into something a developer can act on.
///
/// The failures that actually happen during setup are indistinguishable in a
/// status code and all present as "it silently does not work".
fn explain(status: u16) -> String {
    match status {
        401 => concat!(
            "the API rejected the identity token - check that its ",
            "GOOGLE_CLIENT_ID matches the client this app was built with"
        )
        .into(),
        404 => "no such endpoint - is MEMOS_API_URL the API's base URL?".into(),
        500..=599 => concat!(
            "the API failed - check its logs, and that migrations have been ",
            "applied (scripts/migrate-cloud.ps1)"
        )
        .into(),
        other => format!("the API refused the sign-in ({other})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn session() -> Session {
        Session {
            user_id: "sub-1".into(),
            email: Some("a@b.com".into()),
            display_name: Some("A B".into()),
            access_token: "at".into(),
            id_token: Some("jwt".into()),
            api_token: None,
            api_token_expires_at: None,
            refresh_token: None,
            expires_at: None,
            signed_in_at: Utc::now(),
        }
    }

    fn http() -> reqwest::blocking::Client {
        reqwest::blocking::Client::new()
    }

    #[test]
    fn no_api_configured_is_reported_as_such_rather_than_attempted() {
        let b = Backend::default();
        assert!(!b.is_configured());
        assert!(matches!(
            register(&http(), &b, &session()),
            Err(AuthError::NotConfigured)
        ));
    }

    #[test]
    fn a_whitespace_url_counts_as_no_api() {
        assert!(!Backend { api_url: "   ".into() }.is_configured());
    }

    /// The API authenticates the identity token, not the access token: only the
    /// former carries claims a third party can verify.
    #[test]
    fn a_session_with_no_identity_token_cannot_register() {
        let mut s = session();
        s.id_token = None;
        let b = Backend { api_url: "https://example.invalid".into() };
        assert!(matches!(register(&http(), &b, &s), Err(AuthError::Protocol(_))));
    }

    /// The setup failures that look identical from the outside must not read
    /// identically in the log.
    #[test]
    fn each_setup_failure_names_itself() {
        assert!(explain(401).contains("GOOGLE_CLIENT_ID"));
        assert!(explain(404).contains("MEMOS_API_URL"));
        assert!(explain(503).contains("migrate-cloud"));
        assert!(explain(418).contains("418"));
    }

    #[test]
    fn a_session_response_is_parsed() {
        let raw = r#"{"token":"t","expires_at":"2026-10-01T00:00:00Z",
            "account":{"id":"sub-1","email":"a@b.com","display_name":"A B"}}"#;
        let r: Registered = serde_json::from_str(raw).unwrap();
        assert_eq!(r.token, "t");
        assert_eq!(r.account.email.as_deref(), Some("a@b.com"));
    }
}
