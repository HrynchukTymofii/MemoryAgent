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
    // Declared for every candidate path, including ones that do not exist yet.
    //
    // This is the whole bug it fixes: cargo re-runs a build script when a
    // watched path *changes*, and a path that appears where there was nothing
    // counts. Watching only the file we found meant a checkout with no `.env`
    // watched nothing at all — so creating one afterwards left the binary built
    // from the previous, empty values, reporting "no sign-in credentials" no
    // matter how correct the file was.
    for candidate in dotenv_candidates() {
        println!("cargo:rerun-if-changed={}", candidate.display());
    }
    if let Some(path) = dotenv_candidates().into_iter().find(|p| p.is_file()) {
        load(&path);
    }

    // An absent credential is not an error. A checkout with no `.env` builds a
    // working app with sign-in switched off — which is exactly what a
    // contributor who has never registered an OAuth client should get.
    let mut missing = Vec::new();
    for key in [
        "MEMOS_GOOGLE_CLIENT_ID",
        "MEMOS_GOOGLE_CLIENT_SECRET",
        "MEMOS_API_URL",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        let value = std::env::var(key).unwrap_or_default();
        if value.is_empty() {
            missing.push(key);
        }
        println!("cargo:rustc-env={key}={value}");
    }

    // Said out loud at build time rather than discovered at run time. The
    // symptom of an empty credential is a screen saying sign-in is not
    // configured, which looks like a bug in the app rather than a value that
    // never made it out of a file — so the build names it.
    if !missing.is_empty() {
        println!(
            "cargo:warning=building without {} — sign-in will be switched off.              Copy .env.example to .env at the repository root and fill it in.",
            missing.join(", ")
        );
    }
}

/// Every place a `.env` is looked for: this crate, then outward to the
/// workspace root, which is where it is expected to be.
fn dotenv_candidates() -> Vec<PathBuf> {
    let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") else {
        return Vec::new();
    };
    let here = PathBuf::from(manifest);
    // apps/desktop/src-tauri -> apps/desktop -> apps -> <root>
    here.ancestors().take(4).map(|d| d.join(".env")).collect()
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
