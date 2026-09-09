#!/usr/bin/env bash
# Build a signed, notarised .dmg for distribution.
#
# Everything Apple needs comes from the environment, because these are
# credentials and belong in `.env` (gitignored) rather than in a command
# somebody pastes into a terminal that keeps history.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

[ -f .env ] && set -a && . ./.env && set +a

fail() { printf '\033[31m%s\033[0m\n' "$1" >&2; exit 1; }
note() { printf '\033[36m%s\033[0m\n' "$1"; }

[ "$(uname)" = "Darwin" ] || fail "This builds a macOS bundle; run it on the Mac."

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
