# Referrals in the app: a modal, an account menu, and Pro that lifts the meter

- crates/memos-license/src/lib.rs — new: Entitlement, the weekly limit from it
- crates/memos-auth/src/backend.rs — referral status, apply, invite, progress
- apps/desktop/src-tauri/src/main.rs — four commands; report progress on capture
- apps/desktop/src-tauri/src/config.rs — cache pro_until so the meter is right offline
- apps/desktop/src/features/referral/Referral.tsx — new: the modal, three tabs
- apps/desktop/src/features/account/AccountMenu.tsx — new: popover off the title bar
- apps/desktop/src/hub/App.tsx — Get a free month row; meter reads the entitlement
- apps/desktop/src/lib/api.ts, styles/hub.css — wrappers, styling
