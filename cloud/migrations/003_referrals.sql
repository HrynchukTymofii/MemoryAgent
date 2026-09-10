-- 003_referrals — giving a month of Pro, and getting one back.
--
-- Written to be safe to re-run, like the two before it: this file is applied by
-- hand and the second run must not be an error.

-- What a reward actually grants.
--
-- There is no payment provider yet, so the entitlement is a date rather than a
-- subscription: while `pro_until` is in the future the client lifts its weekly
-- capture limit. When billing arrives this column is what a paid subscription
-- writes to as well, and the client does not have to learn a second answer to
-- "am I on Pro".
alter table users add column if not exists pro_until timestamptz;

-- One live code per user.
--
-- The code is the primary key, not the user: it is what arrives from outside,
-- on a link somebody clicked, and it has to be unique across everyone. A user
-- having exactly one is a `unique` on the other column.
create table if not exists referral_codes (
    code       text primary key,
    user_id    text not null references users(id) on delete cascade,
    created_at timestamptz not null default now()
);
create unique index if not exists idx_referral_codes_user on referral_codes (user_id);

-- Who referred whom.
--
-- `referee_id` is unique, and that single constraint is the anti-abuse rule
-- that matters: an account can be referred exactly once, ever. Without it the
-- same person applies a code, gets their month, and applies the next one.
create table if not exists referrals (
    id           text primary key,
    referrer_id  text not null references users(id) on delete cascade,
    referee_id   text not null unique references users(id) on delete cascade,
    code         text not null,
    -- 'pending' until the referee has actually used the app, then 'qualified'.
    -- A referral that pays out on sign-up alone pays for empty accounts.
    status       text not null default 'pending',
    qualified_at timestamptz,
    created_at   timestamptz not null default now(),
    -- Referring yourself is not a referral. Enforced here rather than only in
    -- the endpoint, because the database is the only layer that cannot be
    -- bypassed by a second code path written later.
    constraint referral_not_self check (referrer_id <> referee_id)
);
create index if not exists idx_referrals_referrer on referrals (referrer_id, created_at desc);

-- Every month ever granted, and what it was for.
--
-- `source_referral_id` is unique so a qualification can only pay out once per
-- side: the grant is an insert, and a repeated `POST /progress` hits the
-- constraint instead of handing out a second month.
create table if not exists rewards (
    id                 text primary key,
    user_id            text not null references users(id) on delete cascade,
    months             smallint not null default 1,
    source_referral_id text not null references referrals(id) on delete cascade,
    -- 'referrer' or 'referee'. Both sides are paid from the same referral, so
    -- the referral id alone does not identify the grant.
    side               text not null,
    granted_at         timestamptz not null default now(),
    unique (source_referral_id, side)
);
create index if not exists idx_rewards_user on rewards (user_id, granted_at desc);

-- Row-level security, in the same shape as `001_users.sql`: a user reaches
-- their own rows and nothing else.
alter table referral_codes enable row level security;
alter table referrals      enable row level security;
alter table rewards        enable row level security;

drop policy if exists referral_codes_own on referral_codes;
create policy referral_codes_own on referral_codes
    for all using (user_id = auth.user_id()) with check (user_id = auth.user_id());

-- Both sides may see a referral they are part of. The referee has to be able to
-- see that their code was applied, or "did that work?" has no answer.
drop policy if exists referrals_own on referrals;
create policy referrals_own on referrals
    for all
    using (referrer_id = auth.user_id() or referee_id = auth.user_id())
    with check (referee_id = auth.user_id());

drop policy if exists rewards_own on rewards;
create policy rewards_own on rewards
    for select using (user_id = auth.user_id());

grant select, insert on referral_codes to authenticated;
grant select, insert on referrals      to authenticated;
grant select          on rewards       to authenticated;

-- Invites sent, so the rate limit has something to count.
--
-- Not a queue and not a delivery log: the mail either went or the endpoint
-- errored. This exists to answer one question — how many has this account sent
-- today — and rows older than a day are of no further interest.
create table if not exists referral_invites (
    id      bigserial primary key,
    user_id text not null references users(id) on delete cascade,
    -- Kept so a second invite to the same address can be recognised later. It
    -- is somebody else's address, given to us to send one message to, and it
    -- is never shown to anyone but the sender.
    email   text not null,
    sent_at timestamptz not null default now()
);
create index if not exists idx_referral_invites_recent on referral_invites (user_id, sent_at desc);

alter table referral_invites enable row level security;
drop policy if exists referral_invites_own on referral_invites;
create policy referral_invites_own on referral_invites
    for all using (user_id = auth.user_id()) with check (user_id = auth.user_id());
grant select, insert on referral_invites to authenticated;
grant usage, select on sequence referral_invites_id_seq to authenticated;
