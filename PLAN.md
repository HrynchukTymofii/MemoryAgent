# Achievements, and a notification centre behind the bell

- crates/memos-db/migrations/004_achievements.sql — daily_activity, achievements, notifications
- crates/memos-db/src/migrate.rs — version 4
- crates/memos-core/src/achieve.rs — new: the milestone ladder, pure
- crates/memos-db/src/stats.rs — new: activity counters, totals, streaks
- crates/memos-db/src/notifications.rs — new: evaluate, list, read, dismiss
- apps/desktop/src-tauri/src/transcription.rs — count words, evaluate after execute
- apps/desktop/src-tauri/src/main.rs — five commands, notification:new event
- apps/desktop/src/features/notifications/ — new: centre, toast
- apps/desktop/src/hub/App.tsx, components/TitleBar.tsx, features/home/Home.tsx — unread count, stat block
- apps/desktop/src/lib/api.ts, styles/hub.css — wrappers, styling
