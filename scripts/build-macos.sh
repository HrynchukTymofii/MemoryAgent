#!/usr/bin/env bash
# Build a signed, notarised .dmg for distribution.
#
# Everything Apple needs comes from the environment, because these are
# credentials and belong in `.env` (gitignored) rather than in a command
# somebody pastes into a terminal that keeps history.
#
#   build-macos.sh            signed and notarised, for other people's Macs
#   build-macos.sh --local    unsigned, for this Mac, no Apple account needed
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

[ -f .env ] && set -a && . ./.env && set +a

fail() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }
note() { printf '\033[36m%s\033[0m\n' "$1"; }

[ "$(uname)" = "Darwin" ] || fail "This builds a macOS bundle; run it on the Mac."

# rustup writes its PATH line into the shell profile, and a profile is only read
# by a login shell — so the first build after installing Rust, and every build
# from a non-interactive shell or a CI step, finds no cargo at all. Sourcing the
# env file directly costs nothing when the PATH is already right.
if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi
command -v cargo >/dev/null || fail "cargo is not on PATH. Install Rust: https://rustup.rs"

# The local build exists because the distribution build cannot be run at all
# without a Developer ID, and "I want to see it on my own Mac" is the first
# thing anybody needs — including on the day the Apple account is still being
# set up. It is deliberately a separate mode rather than a fallback: a build
# that silently stops being notarised is how an unsigned .dmg reaches a user.
if [ "${1:-}" = "--local" ]; then
  arch="$(uname -m)"
  case "$arch" in
    arm64) target="aarch64-apple-darwin" ;;
    x86_64) target="x86_64-apple-darwin" ;;
    *) fail "Unknown architecture: $arch" ;;
  esac

  # Sourcing .env above exports every Apple key in it, and the template ships
  # them empty. Tauri treats "defined but empty" as an instruction to sign,
  # calls codesign with an identity of "", and the build dies at the bundling
  # step after a full compile. Unset rather than skipped: the difference
  # between an empty variable and an absent one is the whole bug.
  unset APPLE_SIGNING_IDENTITY APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD
  unset APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
  unset APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH

  note "Local build for $target — not notarised, so it stays on this Mac."
  cd apps/desktop
  npm ci
  npm run tauri build -- --target "$target" --bundles app

  app="$root/target/$target/release/bundle/macos/PersonalMemoryOS.app"

  # Tauri leaves the bundle with only the linker's ad-hoc signature on the
  # executable, which fails `codesign --verify` and carries no entitlements —
  # so the microphone is denied under the hardened runtime. Signing it here is
  # what makes the local build behave like the shipped one.
  #
  # An Apple Development certificate is preferred over an ad-hoc signature, and
  # not for Gatekeeper's sake — it makes no difference there.
  #
  # It is about TCC. Accessibility and Input Monitoring are remembered against
  # the app's code signature, and an ad-hoc signature is its own hash: every
  # rebuild is a new application that has never been granted anything, so the
  # permission silently stops applying and the shortcut dies until it is granted
  # again. A certificate gives a stable identity, so the grant survives a
  # rebuild — which is the difference between developing this app and fighting
  # System Settings all afternoon.
  identity="$(security find-identity -v -p codesigning \
    | grep -o '"Apple Development: [^"]*"' | head -1 | tr -d '"')"
  if [ -n "$identity" ]; then
    note "Signing with: $identity"
  else
    identity="-"
    note "No Apple Development certificate; signing ad-hoc."
    note "Expect to re-grant Input Monitoring and Accessibility after every rebuild."
  fi

  codesign --force --deep --sign "$identity" --options runtime \
    --entitlements "$root/apps/desktop/src-tauri/entitlements.plist" "$app"
  codesign --verify --deep --strict "$app" || fail "Signature did not verify."

  note "Installing to /Applications..."
  rm -rf "/Applications/PersonalMemoryOS.app"
  cp -R "$app" /Applications/

  note "Done. Launch it with: open -a PersonalMemoryOS"
  note "Gatekeeper will refuse this bundle on any other Mac. That is the point of --local."
  exit 0
fi

# Checked before the build rather than after. A notarisation that fails at the
# end of a fifteen-minute compile because of a missing variable is fifteen
# minutes spent finding that out.
: "${APPLE_SIGNING_IDENTITY:?Set APPLE_SIGNING_IDENTITY, e.g. \"Developer ID Application: Your Name (TEAMID)\"}"
: "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID}"
if [ -z "${APPLE_API_KEY:-}" ]; then
  : "${APPLE_ID:?Set APPLE_ID and APPLE_PASSWORD (an app-specific password), or the APPLE_API_* trio}"
  : "${APPLE_PASSWORD:?APPLE_PASSWORD must be an app-specific password, not your Apple ID password}"
fi

security find-identity -v -p codesigning | grep -q "$APPLE_SIGNING_IDENTITY" \
  || fail "No such signing identity in the keychain: $APPLE_SIGNING_IDENTITY"

note "Models are not bundled — the app downloads them on first run."
note "Building, signing and notarising. Notarisation is Apple's queue; several minutes is normal."

cd apps/desktop
npm ci
npm run tauri build -- --target universal-apple-darwin

out="$root/target/universal-apple-darwin/release/bundle/dmg"
note "Done: $out"

# Proof rather than assumption: an unstapled bundle installs fine on the
# machine that built it and is refused on every other one, which is the worst
# possible way to find out.
app="$root/target/universal-apple-darwin/release/bundle/macos/PersonalMemoryOS.app"
if [ -d "$app" ]; then
  note "Verifying the notarisation ticket is stapled..."
  xcrun stapler validate "$app" || fail "Not stapled. It will be refused on other Macs."
  spctl --assess --type execute --verbose "$app"
fi
