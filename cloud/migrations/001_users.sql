-- 001_users — who is signed in, and nothing else yet.
--
-- Applied to the Neon database the desktop app writes to through the Data API.
-- Kept in the repository rather than clicked into a console so the schema has a
-- history, and so a second environment can be brought up by running it again.
--
-- Every statement is written to be safe to re-run: this file is applied by
-- hand, and the second run must not be an error.

create table if not exists users (
    -- The identity provider's `sub` claim, not an email and not a generated
    -- id. Emails change and people expect to remain themselves when they
    -- change one; `sub` is the only value the provider promises is stable.
    id            text primary key,
    email         text,
    display_name  text,
    -- Updated on every sign-in, which makes it the closest thing to an
    -- activity signal without recording anything about what was captured.
    last_seen_at  timestamptz not null default now(),
    created_at    timestamptz not null default now()
);

-- Row-level security is the whole security model here, not a hardening pass on
-- top of one. The desktop app talks to Postgres directly over HTTPS with a
-- token it holds, so nothing sits between a user and this table that could
-- filter their access — the database has to do it, or nothing does.
alter table users enable row level security;

drop policy if exists users_own_row on users;
create policy users_own_row on users
    for all
    -- `using` governs what an existing row may be read or changed; `with check`
    -- governs what a new or modified row may look like. Both are required:
    -- without the second, a user could insert a row claiming to be someone
    -- else, which is exactly the attack this table has to survive.
    using (id = auth.user_id())
    with check (id = auth.user_id());

-- `authenticated` is the role the Data API assumes once it has validated a
-- token. Anonymous requests get nothing here at all: there is no policy for
-- them and no grant, so an unauthenticated call sees an empty table rather
-- than an error that would confirm the table exists.
grant select, insert, update on users to authenticated;
