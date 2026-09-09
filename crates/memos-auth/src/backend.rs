//! Recording the signed-in user in Postgres.
//!
//! ## Why there is no connection string
//!
//! A desktop application cannot hold a database password. It ships to everyone,
//! it is readable by everyone, and a Postgres URL inside it is a credential
//! granting whoever extracts it full access to every row every user has ever
//! written. There is no configuration that makes that safe.
//!
//! So the app never speaks Postgres. It speaks HTTPS to Neon's Data API — a
//! PostgREST-compatible endpoint in front of the same database — and
//! authenticates with the OIDC identity token it already holds from signing in.
//! Neon validates that token against the provider's public keys and runs the
//! statement as that user, so row-level security decides what a request may
//! touch. A stolen token is one user's session and expires; a stolen connection
//! string is the whole database forever.
//!
//! ```text
//! desktop  --id_token-->  Neon Data API  --RLS-->  Postgres
//! ```
//!
//! The table this expects, and the policies that make it safe, are in the
//! Account section of the README.

use serde::Serialize;

use crate::{AuthError, AuthResult, Session};

/// Where the backend lives. Empty means there is none, which is a supported
/// state: the app keeps the session locally and records nothing.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct Backend {
    /// Base URL of the Data API, without a trailing slash.
    #[serde(default)]
    pub data_api_url: String,
}

impl Backend {
    pub fn is_configured(&self) -> bool {
        !self.data_api_url.trim().is_empty()
    }
}

/// One row of the `users` table, as the app knows it.
#[derive(Debug, Serialize)]
struct UserRow<'a> {
    /// The provider's subject claim. The primary key, because it is the only
    /// identifier that is stable when someone changes their email address.
    id: &'a str,
    email: Option<&'a str>,
    display_name: Option<&'a str>,
    last_seen_at: String,
}

/// Write the signed-in user into Postgres, creating or updating the row.
///
/// Deliberately not fatal. Sign-in has already succeeded by the time this runs
/// — the user is signed in, locally, whatever happens here — so a backend that
/// is unreachable produces a warning and nothing else. Failing the sign-in over
/// a bookkeeping write would be losing the thing the user asked for in order to
/// protect the thing they did not.
pub fn record_user(
    http: &reqwest::blocking::Client,
    backend: &Backend,
    session: &Session,
) -> AuthResult<()> {
    if !backend.is_configured() {
        return Ok(());
    }
    let Some(token) = session.id_token.as_deref() else {
        // Nothing to authenticate with. A provider that issued no identity
        // token cannot be used against a database that authenticates with one.
        return Err(AuthError::Protocol(
            "no identity token to authenticate with".into(),
        ));
    };
    if session.user_id.is_empty() {
        return Err(AuthError::Protocol("session has no user id".into()));
    }

    let row = UserRow {
        id: &session.user_id,
        email: session.email.as_deref(),
        display_name: session.display_name.as_deref(),
        last_seen_at: chrono::Utc::now().to_rfc3339(),
    };

    let url = format!("{}/users", backend.data_api_url.trim_end_matches('/'));
    let response = http
        .post(&url)
        .bearer_auth(token)
        // PostgREST's upsert: insert, and on a primary-key collision update
        // instead. Without it a returning user is a duplicate-key error on
        // every launch — which is the normal case, not the exception.
        .header("Prefer", "resolution=merge-duplicates,return=minimal")
        .json(&row)
        .send()
        .map_err(|e| AuthError::Network(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        tracing::warn!(%status, body = %body.chars().take(300).collect::<String>(),
            "could not record the user");
        return Err(AuthError::Protocol(format!("backend refused the write ({status})")));
    }
    tracing::info!(user = %session.user_id, "recorded the signed-in user");
    Ok(())
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
            refresh_token: None,
            expires_at: None,
            signed_in_at: Utc::now(),
        }
    }

    fn http() -> reqwest::blocking::Client {
        reqwest::blocking::Client::new()
    }

    #[test]
    fn no_backend_configured_is_a_no_op_rather_than_an_error() {
        let b = Backend::default();
        assert!(!b.is_configured());
        record_user(&http(), &b, &session()).unwrap();
    }

    #[test]
    fn a_whitespace_url_counts_as_no_backend() {
        assert!(!Backend { data_api_url: "   ".into() }.is_configured());
    }

    /// The database authenticates with the identity token, not the access
    /// token, and a session without one cannot write at all.
    #[test]
    fn a_session_with_no_identity_token_cannot_write() {
        let mut s = session();
        s.id_token = None;
        let b = Backend { data_api_url: "https://example.invalid".into() };
        assert!(matches!(record_user(&http(), &b, &s), Err(AuthError::Protocol(_))));
    }

    #[test]
    fn a_session_with_no_user_id_cannot_write() {
        let mut s = session();
        s.user_id = String::new();
        let b = Backend { data_api_url: "https://example.invalid".into() };
        assert!(matches!(record_user(&http(), &b, &s), Err(AuthError::Protocol(_))));
    }

    /// The row is keyed by the provider's subject, never by email — people
    /// change their address and must stay the same user when they do.
    #[test]
    fn the_row_is_keyed_by_subject_not_by_email() {
        let s = session();
        let row = UserRow {
            id: &s.user_id,
            email: s.email.as_deref(),
            display_name: s.display_name.as_deref(),
            last_seen_at: "2026-01-01T00:00:00Z".into(),
        };
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["id"], "sub-1");
        assert_eq!(json["email"], "a@b.com");
    }
}
