-- 002_email_codes — signing in with an email address.

create table if not exists email_codes (
    email        text primary key,
    -- The code is stored as a SHA-256 hash, never in clear. A leaked snapshot
    -- of this table must not be a list of working sign-in codes.
    code_hash    text        not null,
    expires_at   timestamptz not null,
    -- Counted so a code can be guessed a few times and then not at all. Six
    -- digits is a million possibilities; unlimited attempts makes that a
    -- number a script gets through in seconds.
    attempts     smallint    not null default 0,
    created_at   timestamptz not null default now()
);

-- One live code per address: requesting a new one replaces the old, so an
-- attacker cannot accumulate valid codes by asking repeatedly.
create index if not exists idx_email_codes_expiry on email_codes (expires_at);
