#!/usr/bin/env bash
# Build the Tier 1 router sidecar. The macOS and Linux counterpart of
# build-router.ps1.
#
#   ./scripts/build-router.sh           # release, and install it into the app
#   ./scripts/build-router.sh --debug   # debug, for `tauri dev`
#
# Separate from the app's own build on purpose (ADR-0008). This is the only
# binary in the workspace that links llama.cpp, and llama.cpp cannot be linked
# into the app: whisper.cpp vendors its own copy of ggml, so one executable
# containing both defines every ggml symbol twice and fails to link.
#
# Without it the app still runs — Tier 0 answers the formulaic commands and
# unusual phrasings come back "not sure what to do with that", which is exactly
# the behaviour before Tier 1 existed.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fail() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }
note() { printf '\033[36m%s\033[0m\n' "$1"; }
ok()   { printf '\033[32m%s\033[0m\n' "$1"; }

profile="release"
[ "${1:-}" = "--debug" ] && profile="debug"

command -v cmake >/dev/null || fail "llama.cpp needs CMake. brew install cmake"

# Built for the same target the app is, so the sidecar that ends up beside the
# app binary is the same architecture as the process that spawns it.
case "$(uname -m)" in
  arm64)  target="aarch64-apple-darwin" ;;
  x86_64) target="x86_64-apple-darwin" ;;
  *)      fail "Unknown architecture: $(uname -m)" ;;
esac

note "Building memos-router ($profile). First run compiles llama.cpp; expect a few minutes."
note "Metal is on by default on Apple silicon — the router runs on the GPU."

if [ "$profile" = "release" ]; then
    cargo build --manifest-path "$root/Cargo.toml" --release --target "$target" \
        -p memos-llm --features local --bin memos-router
else
    cargo build --manifest-path "$root/Cargo.toml" --target "$target" \
        -p memos-llm --features local --bin memos-router
fi

exe="$root/target/$target/$profile/memos-router"
[ -f "$exe" ] || fail "cargo reported success but $exe is not there."
ok "Done: $exe ($(du -m "$exe" | cut -f1) MB)"

# The app looks for the sidecar beside its own binary, which inside a bundle
# means Contents/MacOS. Copying it in is what makes an installed app able to
# find it at all — `target/` is not a place a shipped app can reach.
app="/Applications/PersonalMemoryOS.app"
if [ -d "$app" ]; then
    cp "$exe" "$app/Contents/MacOS/memos-router"

    # Adding a file to a signed bundle invalidates its seal, so the whole bundle
    # is signed again rather than just the new binary. Skipping this leaves an
    # app that launches from Finder today and is refused after the next reboot,
    # which is a miserable thing to debug.
    identity="$(codesign -dv --verbose=2 "$app" 2>&1 | grep '^Authority=' | head -1 | cut -d= -f2-)"
    [ -n "$identity" ] || identity="-"
    codesign --force --deep --sign "$identity" --options runtime \
        --entitlements "$root/apps/desktop/src-tauri/entitlements.plist" "$app"
    codesign --verify --deep --strict "$app" || fail "Re-signing the app failed."
    ok "Installed into $app and re-signed ($identity)"
fi

# Checked here rather than left to the app, because a router binary with no
# model is a silent no-op: the app starts, Tier 1 reports "missing", and the
# only symptom is that unusual phrasings keep failing.
if [ ! -f "$root/models/llm/router.gguf" ]; then
    printf '\033[33m%s\033[0m\n' "No model yet. Fetch it: ./scripts/fetch-models.sh router"
fi
