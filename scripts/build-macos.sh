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

  note "Local build for $target — unsigned, and it will not run on another Mac."
  cd apps/desktop
  npm ci
  npm run tauri build -- --target "$target" --bundles app

  app="$root/target/$target/release/bundle/macos/PersonalMemoryOS.app"

  # Tauri leaves the bundle with only the linker's ad-hoc signature on the
  # executable, which fails `codesign --verify` and carries no entitlements —
  # so the microphone is denied under the hardened runtime. Signing it here
  # ad-hoc is what makes the local build behave like the shipped one.
  note "Ad-hoc signing with the real entitlements..."
  codesign --force --deep --sign - --options runtime \
    --entitlements "$root/apps/desktop/src-tauri/entitlements.plist" "$app"
  codesign --verify --deep --strict "$app" || fail "Ad-hoc signature did not verify."

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
