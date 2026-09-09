# ADR-0009: The macOS port is two files and three permissions

**Status:** Accepted · **Date:** 2026-09-09 · **Refines:** the M7 roadmap entry

## Context

The roadmap puts macOS at M7 on the grounds that "only `memos-context` needs a
new implementation". That claim was never checked. Checking it before starting
the port is cheap; discovering it was wrong halfway through is not.

It holds. Everything Windows-specific in the repository is already behind
`cfg(windows)` with a compiling fallback on the other side, so the workspace
builds for macOS today — it simply captures no context and installs no hotkey.

Measured, rather than assumed:

| | |
|---|---|
| `crates/memos-context/src/windows_impl.rs` | 385 lines, UI Automation |
| `apps/desktop/src-tauri/src/hotkey.rs` (`imp`) | ~180 lines, `SetWindowsHookExW` |
| `cfg(windows)` blocks in `main.rs`, `router.rs` | 5 |
| **Platform-neutral and untouched** | everything else, including `readable.rs` |

`readable.rs` matters most in that table. The text-density trimming is the
hardest-won code in the capture path and it is pure string processing, so it
survives the port unchanged.

## Decision

### The port is two implementations, not a rewrite

1. **`memos-context`** — replace the `cfg(not(windows))` stub with the
   Accessibility API: `AXUIElementCopyAttributeValue` for the focused window's
   title, `kAXSelectedTextAttribute` for the selection, and the browser URL from
   `kAXDocumentAttribute` or Apple Events. The `Context` struct, the staged
   collection deadline and the trimming are all unchanged.

2. **The hotkey** — replace the no-op `install()` with a `CGEventTap` on
   `kCGEventKeyDown`/`kCGEventFlagsChanged`. The channel contract
   (`ChordState` in, `Receiver` out) is platform-neutral, and `Chord` already
   parses and matches without reference to Windows virtual-key codes.

### Distribution is Developer ID, not the Mac App Store

The App Store requires the app sandbox, and the sandbox forbids the
Accessibility API this product reads context through. A sandboxed build cannot
see the window you are looking at, which is the feature. So:
`entitlements.plist` asks for audio input and network client, and deliberately
does **not** ask for `com.apple.security.app-sandbox`.

This also settles the code-signing question that Windows left open: an Apple
Developer account covers Developer ID signing and notarisation, so macOS ships
without the separate certificate purchase Windows needs.

### Three permissions, and two of them cannot be asked for in code

- **Microphone** — prompted on first use, but only if
  `NSMicrophoneUsageDescription` is in `Info.plist`. Without it the process is
  killed rather than shown a prompt.
- **Accessibility** — no API prompts for this. The user must add the app in
  System Settings → Privacy & Security → Accessibility, and the app can only
  detect that it does not have it. Onboarding has to walk them there; this is
  the largest user-facing difference from Windows, where a low-level hook needs
  no permission at all.
- **Input Monitoring** — required for a `CGEventTap` and granted separately from
  Accessibility, which is a distinction that will be discovered the hard way if
  it is not written down here.

## Consequences

**Good**

- The crate boundaries were drawn correctly, and this ADR is the evidence.
- No Windows code has to be untangled first: the split already exists.
- Notarisation replaces a code-signing certificate that would otherwise have to
  be bought and stored on a hardware token.

**Bad**

- Two permissions require a trip to System Settings that cannot be automated,
  and one of them is invisible until the shortcut silently fails to fire.
- Neither implementation can be compiled or tested from a Windows machine, so
  they are the one part of this repository that must be written at a Mac.

**Neutral**

- `LSUIElement` makes it a menu-bar application with no Dock icon, matching the
  tray behaviour on Windows.
