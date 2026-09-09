# Fix: "Sign in with Google" opens Explorer instead of the browser

`explorer.exe <url>` mangles URLs with a query string and opens a folder.
Use `ShellExecuteW` instead.

- `crates/memos-auth/Cargo.toml` — add `windows` crate for the Windows target
- `crates/memos-auth/src/lib.rs` — `open_browser` uses `ShellExecuteW` on Windows
