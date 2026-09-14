# A capture held longer than 30 s is transcribed, not dropped

- crates/memos-stt/src/ring.rs — `drain()`: take what arrived since a cursor and advance it; tests
- apps/desktop/src-tauri/src/main.rs — while the chord is held, drain the ring every few seconds into the capture
