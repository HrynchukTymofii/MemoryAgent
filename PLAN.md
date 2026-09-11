# Route every command through a cloud tool-calling agent; Tier 0 becomes the offline fallback

- crates/memos-cloud/ — new: tool schemas, Claude Messages call, tool loop.
- crates/memos-core/src/intent.rs — `CreateCollection`.
- crates/memos-agent/src/execute.rs — execute `CreateCollection`.
- apps/desktop/src-tauri/src/transcription.rs — cloud first, grammar on failure; Tier 1 out.
- apps/desktop/src-tauri/src/main.rs — no sidecar; `router_status` reports the cloud tier.
- apps/desktop/src-tauri/src/config.rs — `anthropic_api_key`.
- apps/desktop/src/features/settings/Settings.tsx — the Router pill reads the cloud tier.
- Cargo.toml, apps/desktop/src-tauri/Cargo.toml — the new crate.
- .env.example, docs/adr/0011-cloud-first-routing.md — the key, and why the tiers went.
