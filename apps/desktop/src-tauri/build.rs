use std::path::{Path, PathBuf};

fn main() {
    bake_in_oauth_credentials();
    tauri_build::build()
}

/// Compile the app's own OAuth credentials into the binary.
///
/// These identify *this application* to Google, not the person using it. They
/// are the same for every install, they belong to whoever ships the app, and no
/// user should ever see or type them — which is why they arrive at build time
/// from a file in the repository rather than from anything on the user's
/// machine.
///
/// Google documents the client secret for an installed app as not confidential:
/// it ships inside every copy of the binary and cannot be otherwise. PKCE is
/// what actually protects the exchange. It is kept out of git all the same,
/// because a credential in a public history is a credential you have to rotate.
fn bake_in_oauth_credentials() {
    let env_file = find_dotenv();
    if let Some(path) = &env_file {
        println!("cargo:rerun-if-changed={}", path.display());
        load(path);
    }

    // An absent credential is not an error. A checkout with no `.env` builds a
    // working app with sign-in switched off — which is exactly what a
    // contributor who has never registered an OAuth client should get.
    for key in [
        "MEMOS_GOOGLE_CLIENT_ID",
        "MEMOS_GOOGLE_CLIENT_SECRET",
        "MEMOS_DATA_API_URL",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        let value = std::env::var(key).unwrap_or_default();
        println!("cargo:rustc-env={key}={value}");
    }
}

/// `.env` beside the workspace root, then beside this crate.
fn find_dotenv() -> Option<PathBuf> {
    let here = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    // apps/desktop/src-tauri -> apps/desktop -> apps -> <root>
    let candidates = [
        here.join(".env"),
        here.parent()?.join(".env"),
        here.ancestors().nth(3)?.join(".env"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// Minimal `KEY=value` reader.
///
/// A dependency for this would be a build-time dependency on every machine that
/// compiles the app, to parse a file with two lines in it. Values already in the
/// environment win, so CI can pass them without a file.
fn load(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    }
}
