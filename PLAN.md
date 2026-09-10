# Collections become a place you walk into, not a list you read

- crates/memos-db/src/repo.rs — `rename_collection` (re-materialises the
  descendants' paths), `delete_collection` (cascade takes children; items fall
  back to unfiled), `count_in_subtree` for the confirmation.
- apps/desktop/src-tauri/src/main.rs — the three as commands, parented by path.
- apps/desktop/src/lib/api.ts — the three wrappers.
- apps/desktop/src/features/collections/Collections.tsx — one level at a time:
  breadcrumb, a grid of folder cards, the memories filed here under it. Rename
  and delete per card, a New tile at the end.
- apps/desktop/src/styles/hub.css — the grid, cards, breadcrumb.
