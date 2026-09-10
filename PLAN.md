# Task rows get smaller, gain edit/delete, and say when they are due

- crates/memos-db/src/actions.rs — `update_task` (title, due_at), `delete_task`.
- apps/desktop/src-tauri/src/main.rs — same two as commands; `TaskRow` drops `about`.
- apps/desktop/src/lib/api.ts — the two wrappers; `TaskRow.about` goes.
- apps/desktop/src/features/tasks/Tasks.tsx — one-line row: box, title, due
  chip, created stamp, hover edit/delete. Inline rename. Overdue is a state.
- apps/desktop/src/styles/hub.css — the row shrinks; chip and row actions.
