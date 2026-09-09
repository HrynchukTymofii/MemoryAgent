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
