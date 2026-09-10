# Track silently: drop the words/streak block and its chart from Home

- apps/desktop/src/features/home/Home.tsx — tally block and Fortnight gone
- apps/desktop/src/lib/api.ts — Stats and achievementStats gone
- apps/desktop/src-tauri/src/main.rs — the achievement_stats command gone
- crates/memos-db/src/stats.rs — recent_days and words_today, which only fed it
- apps/desktop/src/styles/hub.css — .spark only; .tally stays for Past referrals
