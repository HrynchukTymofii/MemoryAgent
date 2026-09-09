# Email sign-in (6-digit code)

Backend sends a code, user types it in the app. No redirect, no browser.

- `cloud/migrations/002_email_codes.sql` — new: codes table, hashed, expiring
- `cloud/app/email.py` — new: SMTP send, works with Resend/Postmark/SES/Gmail
- `cloud/app/codes.py` — new: generate, hash, verify, rate-limit
- `cloud/app/db.py` — store + consume a code
- `cloud/app/main.py` — `POST /v1/auth/email/start`, `/v1/auth/email/verify`
- `cloud/app/config.py` — SMTP settings
- `crates/memos-auth/src/backend.rs` — `email_start`, `email_verify`
- `apps/desktop/src-tauri/src/main.rs` — two commands
- `apps/desktop/src/features/account/SignIn.tsx` — email field + code field
- `.env.example`, `cloud/README.md` — SMTP vars
