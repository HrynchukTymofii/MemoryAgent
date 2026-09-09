//! The signed-in session, and where it is kept between launches.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{AuthError, AuthResult};

/// Who is signed in, and what we hold on their behalf.
///
/// The tokens are secrets and the identity is not, so the two are read for very
/// different reasons: `email` exists to be shown in the interface and counted
/// as a metric, the tokens exist only to be sent back to the provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,

    pub access_token: String,
    /// The OIDC identity token, kept because it is the credential the backend
    /// accepts: Postgres validates it against the provider's JWKS and scopes
    /// rows by its `sub` claim. Google's access tokens are opaque and cannot do
    /// that job, so this is not a duplicate of the field above.
    #[serde(default)]
    pub id_token: Option<String>,
    /// Absent when the provider issues none, which makes the session last
    /// exactly as long as the access token does.
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,

    /// When this session was first established. Kept because "signed in since"
    /// is the honest thing to show, and because it survives a token refresh
    /// while `expires_at` does not.
    pub signed_in_at: DateTime<Utc>,
}

impl Session {
    /// Whether the access token needs refreshing before it is used.
    ///
    /// Early by a minute. A token that expires between the check and the
    /// request arriving is a spurious failure the user cannot act on, and a
    /// minute costs nothing.
    pub fn needs_refresh(&self) -> bool {
        match self.expires_at {
            Some(at) => Utc::now() + Duration::seconds(60) >= at,
            // No expiry given means no way to know it has expired. Treating it
            // as fresh is right: the alternative is refreshing on every call.
            None => false,
        }
    }
}

/// What the interface is allowed to know about the session.
///
/// A separate type from [`Session`] on purpose. Everything the Hub needs is
/// here and no token is, so there is no path by which an access token reaches a
/// webview — the boundary is enforced by the type rather than by remembering to
/// strip fields at each call site.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Identity {
    pub signed_in: bool,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub signed_in_at: Option<DateTime<Utc>>,
}

impl Identity {
    pub fn anonymous() -> Self {
        Self {
            signed_in: false,
            email: None,
            display_name: None,
            signed_in_at: None,
        }
    }
}

impl From<&Session> for Identity {
    fn from(s: &Session) -> Self {
        Self {
            signed_in: true,
            email: s.email.clone(),
            display_name: s.display_name.clone(),
            signed_in_at: Some(s.signed_in_at),
        }
    }
}

/// Where the session lives on disk.
///
/// A plain file beside the database, holding a refresh token in clear text.
/// That is worth stating rather than glossing: anything on this machine running
/// as this user can read it. It is the same protection the database itself has,
/// which already holds every memory the user has ever captured — so the file is
/// not the weak link, and encrypting it with a key stored next to it would be
/// theatre.
///
/// The real improvement is DPAPI on Windows and the Keychain on macOS, which
/// bind the secret to the user account rather than the file system. That is a
/// deliberate follow-up, not an oversight; it is noted here so the next person
/// does not have to work out whether it was considered.
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("session.json"),
        }
    }

    /// Load, treating a corrupt file as "signed out".
    ///
    /// Never an error. A session that cannot be read is one the user has to
    /// establish again, and refusing to start the app over it would turn a
    /// recoverable annoyance into a broken install.
    pub fn load(&self) -> Option<Session> {
        let raw = std::fs::read_to_string(&self.path).ok()?;
        match serde_json::from_str(raw.trim_start_matches('\u{feff}')) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(?e, "session file unreadable; treating as signed out");
                None
            }
        }
    }

    pub fn save(&self, session: &Session) -> AuthResult<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AuthError::Store(e.to_string()))?;
        }
        let json =
            serde_json::to_string_pretty(session).map_err(|e| AuthError::Store(e.to_string()))?;
        std::fs::write(&self.path, json).map_err(|e| AuthError::Store(e.to_string()))?;
        restrict(&self.path);
        Ok(())
    }

    /// Forget the session.
    ///
    /// Removing the file rather than blanking it: a file of empty strings is
    /// indistinguishable from a corrupt one, and this has to be unambiguous.
    pub fn clear(&self) -> AuthResult<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            // Already gone is the state we wanted.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AuthError::Store(e.to_string())),
        }
    }
}

/// Narrow the file's permissions where the platform makes that cheap.
#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// On Windows the file inherits the user profile's ACL, which already excludes
/// other users. Tightening it further would need a hand-built security
/// descriptor for no gain over what the profile directory provides.
#[cfg(not(unix))]
fn restrict(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            user_id: "u_1".into(),
            email: Some("someone@example.com".into()),
            display_name: Some("Someone".into()),
            access_token: "ACCESS-SECRET".into(),
            id_token: Some("ID-SECRET".into()),
            refresh_token: Some("REFRESH-SECRET".into()),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            signed_in_at: Utc::now(),
        }
    }

    #[test]
    fn a_session_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let s = session();
        store.save(&s).unwrap();
        assert_eq!(store.load().unwrap(), s);
    }

    #[test]
    fn no_session_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SessionStore::new(dir.path()).load().is_none());
    }

    /// A half-written file must read as signed out, not as a startup failure.
    #[test]
    fn a_corrupt_session_reads_as_signed_out() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        std::fs::write(dir.path().join("session.json"), "{ not json").unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn clearing_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store.save(&session()).unwrap();
        store.clear().unwrap();
        store.clear().unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn a_token_near_its_expiry_is_refreshed_early() {
        let mut s = session();
        s.expires_at = Some(Utc::now() + Duration::seconds(30));
        assert!(s.needs_refresh(), "30s left is inside the one-minute margin");
        s.expires_at = Some(Utc::now() + Duration::hours(1));
        assert!(!s.needs_refresh());
    }

    #[test]
    fn a_token_with_no_expiry_is_never_refreshed() {
        let mut s = session();
        s.expires_at = None;
        assert!(!s.needs_refresh());
    }

    /// The type the interface sees must not be able to carry a token at all.
    #[test]
    fn identity_carries_no_secrets() {
        let s = session();
        let json = serde_json::to_string(&Identity::from(&s)).unwrap();
        assert!(json.contains("someone@example.com"), "the email is the point");
        assert!(!json.contains("SECRET"), "a token reached the interface: {json}");
    }
}
