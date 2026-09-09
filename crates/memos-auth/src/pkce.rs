//! PKCE, and the state parameter that goes with it.
//!
//! A desktop application cannot keep a secret. Anything compiled into the
//! binary is readable by anyone who has the binary, which is everyone who
//! installs it — so the classic client-secret exchange is not available here,
//! and pretending otherwise would be worse than not having one at all.
//!
//! PKCE (RFC 7636) replaces the secret with one the client invents per attempt:
//! a random verifier is held in memory, only its SHA-256 hash is sent with the
//! authorisation request, and the verifier itself is revealed only when
//! redeeming the code. An attacker who intercepts the redirect — which on a
//! loopback address is a real possibility, since any local process can race for
//! the port — has a code they cannot spend.

use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

/// A verifier and the challenge derived from it, for one sign-in attempt.
#[derive(Debug, Clone)]
pub struct Pkce {
    /// Held in memory only, and sent exactly once, at the token exchange.
    pub verifier: String,
    /// `BASE64URL(SHA256(verifier))` — safe to put in a URL the browser sees.
    pub challenge: String,
    /// Ties the callback to this attempt. Without it, a callback arriving at
    /// our loopback port from anywhere at all would be treated as the answer to
    /// the request we made.
    pub state: String,
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = random_token(64);
        let digest = Sha256::digest(verifier.as_bytes());
        Self {
            challenge: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest),
            verifier,
            state: random_token(24),
        }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// `len` bytes from the OS random source, base64url-encoded.
///
/// `rand::thread_rng` is seeded from the operating system and reseeded
/// periodically; the encoding is unpadded because both values end up in URLs
/// where `=` would have to be escaped.
fn random_token(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill(&mut bytes[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_the_hash_of_the_verifier() {
        let p = Pkce::new();
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(p.verifier.as_bytes()));
        assert_eq!(p.challenge, expected);
        assert_ne!(p.challenge, p.verifier, "the verifier must not be sent");
    }

    /// RFC 7636 requires 43-128 characters. Shorter is guessable; longer is
    /// rejected by some servers.
    #[test]
    fn the_verifier_is_within_the_length_the_spec_allows() {
        let p = Pkce::new();
        assert!((43..=128).contains(&p.verifier.len()), "{}", p.verifier.len());
    }

    #[test]
    fn every_attempt_is_unique() {
        let (a, b) = (Pkce::new(), Pkce::new());
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state, "a reused state would accept a replayed callback");
    }

    /// These go into a query string unescaped, so they must contain nothing
    /// that would need escaping.
    #[test]
    fn both_values_are_url_safe() {
        let p = Pkce::new();
        for token in [&p.verifier, &p.challenge, &p.state] {
            assert!(
                token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{token}"
            );
        }
    }
}
