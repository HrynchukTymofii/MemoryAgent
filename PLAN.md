# Referrals: codes, invites, and a month of Pro for both sides

- cloud/migrations/003_referrals.sql — codes, referrals, rewards, users.pro_until
- cloud/app/referrals.py — new: code generation, qualification, granting months
- cloud/app/db.py — the queries behind them
- cloud/app/mail.py — the invite mail
- cloud/app/config.py — download_url, referral_qualify_words, reward_months
- cloud/app/main.py — /v1/referrals/{me,apply,invite,progress}, GET /r/{code}
- cloud/tests/test_referrals.py — self-referral, double-apply, granted once
