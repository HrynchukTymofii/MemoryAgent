"""The connection pool, and every query the service runs.

SQL is written here rather than through an ORM. The schema is in
`cloud/migrations/` where it can be read, and an ORM would add a layer of
indirection over statements that are mostly shorter than their own mapping
would be.

The referral queries at the bottom are the exception to "one statement, one
method": granting a month has to read an entitlement, extend it, and record why
— and those three must not interleave with another request doing the same. They
run in one transaction, and the reads inside it take a row lock.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from typing import Any

from psycopg.rows import dict_row
from psycopg_pool import AsyncConnectionPool


@dataclass(frozen=True)
class User:
    id: str
    email: str | None
    display_name: str | None
    created_at: datetime
    last_seen_at: datetime
    #: While this is in the future the client lifts its weekly capture limit.
    pro_until: datetime | None = None

    @classmethod
    def of(cls, row: dict[str, Any]) -> "User":
        return cls(
            id=row["id"],
            email=row["email"],
            display_name=row["display_name"],
            created_at=row["created_at"],
            last_seen_at=row["last_seen_at"],
            # Absent from the rows returned by the sign-in statements, which do
            # not select it: nobody is told their plan by the act of signing in.
            pro_until=row.get("pro_until"),
        )


@dataclass(frozen=True)
class Referral:
    """One person brought in by another, as the referrer sees it."""

    id: str
    #: Who was referred. Shown as an initial and a domain, never in full — see
    #: `main.py`; the referrer is owed a count, not somebody's address.
    referee_email: str | None
    status: str
    created_at: datetime
    qualified_at: datetime | None


class Database:
    """Owns the pool. One instance, created at startup and closed at shutdown."""

    def __init__(self, url: str) -> None:
        # Opened lazily so importing this module never touches the network —
        # which is what lets the tests import it without a database.
        self._pool = AsyncConnectionPool(url, open=False, min_size=1, max_size=10)

    async def open(self) -> None:
        await self._pool.open(wait=True)

    async def close(self) -> None:
        await self._pool.close()

    async def upsert_user(
        self, *, sub: str, email: str | None, display_name: str | None
    ) -> User:
        """Record a sign-in, creating the user the first time.

        `last_seen_at` is always bumped; `email` and `display_name` are only
        overwritten when the provider actually sent them, so a token that omits
        a field cannot blank a value we already had.
        """
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    insert into users (id, email, display_name, last_seen_at)
                    values (%s, %s, %s, now())
                    on conflict (id) do update set
                        email        = coalesce(excluded.email, users.email),
                        display_name = coalesce(excluded.display_name, users.display_name),
                        last_seen_at = now()
                    returning id, email, display_name, created_at, last_seen_at
                    """,
                    (sub, email, display_name),
                )
                row = await cur.fetchone()
        assert row is not None  # `returning` on an upsert always yields a row
        return User.of(row)

    async def store_code(self, *, email: str, code_hash: str, ttl_minutes: int) -> None:
        """Replace any live code for this address with a new one.

        Replace, not add. Otherwise asking for a code repeatedly accumulates
        valid ones, and the attempt limit — which is what makes six digits safe
        — is applied per row rather than per address.
        """
        async with self._pool.connection() as conn:
            await conn.execute(
                """
                insert into email_codes (email, code_hash, expires_at, attempts)
                values (%s, %s, now() + make_interval(mins => %s), 0)
                on conflict (email) do update set
                    code_hash  = excluded.code_hash,
                    expires_at = excluded.expires_at,
                    attempts   = 0,
                    created_at = now()
                """,
                (email, code_hash, ttl_minutes),
            )

    async def take_code(self, *, email: str, max_attempts: int) -> str | None:
        """Claim the live code for an address, counting the attempt.

        The count is incremented in the same statement that reads the row, so
        two requests racing cannot both see the same attempt number. Returns
        the stored hash for the caller to compare, or `None` when there is no
        live code left to try.
        """
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    update email_codes
                       set attempts = attempts + 1
                     where email = %s
                       and expires_at > now()
                       and attempts < %s
                    returning code_hash
                    """,
                    (email, max_attempts),
                )
                row = await cur.fetchone()
        return row["code_hash"] if row else None

    async def clear_code(self, email: str) -> None:
        """Spend the code. A correct one must not work twice."""
        async with self._pool.connection() as conn:
            await conn.execute("delete from email_codes where email = %s", (email,))

    async def user_by_email(self, email: str) -> User | None:
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    "select id, email, display_name, created_at, last_seen_at "
                    "from users where lower(email) = %s",
                    (email,),
                )
                row = await cur.fetchone()
        return User.of(row) if row else None

    async def user(self, sub: str) -> User | None:
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    "select id, email, display_name, created_at, last_seen_at "
                    "from users where id = %s",
                    (sub,),
                )
                row = await cur.fetchone()
        return User.of(row) if row else None

    # ------------------------------------------------------------ referrals

    async def referral_code(self, user_id: str) -> str | None:
        """The user's code, if they have been given one."""
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    "select code from referral_codes where user_id = %s", (user_id,)
                )
                row = await cur.fetchone()
        return row["code"] if row else None

    async def claim_referral_code(self, *, user_id: str, code: str) -> str | None:
        """Take `code` for this user, or `None` if somebody already has it.

        Two conflicts, two meanings. A clash on `code` is a collision and the
        caller should try another; a clash on the user's unique index means they
        already had one, and the right answer is the code they already have —
        never a second, because the first is already in somebody's inbox.
        """
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    insert into referral_codes (code, user_id)
                    values (%s, %s)
                    on conflict do nothing
                    returning code
                    """,
                    (code, user_id),
                )
                row = await cur.fetchone()
                if row:
                    return row["code"]
                await cur.execute(
                    "select code from referral_codes where user_id = %s", (user_id,)
                )
                mine = await cur.fetchone()
        return mine["code"] if mine else None

    async def user_by_referral_code(self, code: str) -> User | None:
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    select u.id, u.email, u.display_name, u.created_at,
                           u.last_seen_at, u.pro_until
                      from referral_codes c
                      join users u on u.id = c.user_id
                     where c.code = %s
                    """,
                    (code,),
                )
                row = await cur.fetchone()
        return User.of(row) if row else None

    async def record_referral(
        self, *, referral_id: str, referrer_id: str, referee_id: str, code: str
    ) -> bool:
        """Attach the referee to the referrer. False if they were already
        referred by anyone, which is the once-ever rule the unique index holds.
        """
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    insert into referrals (id, referrer_id, referee_id, code)
                    values (%s, %s, %s, %s)
                    on conflict (referee_id) do nothing
                    returning id
                    """,
                    (referral_id, referrer_id, referee_id, code),
                )
                row = await cur.fetchone()
        return row is not None

    async def referrals_by(self, referrer_id: str) -> list[Referral]:
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    select r.id, r.status, r.created_at, r.qualified_at,
                           u.email as referee_email
                      from referrals r
                      join users u on u.id = r.referee_id
                     where r.referrer_id = %s
                     order by r.created_at desc
                    """,
                    (referrer_id,),
                )
                rows = await cur.fetchall()
        return [
            Referral(
                id=r["id"],
                referee_email=r["referee_email"],
                status=r["status"],
                created_at=r["created_at"],
                qualified_at=r["qualified_at"],
            )
            for r in rows
        ]

    async def pending_referral_of(self, referee_id: str) -> tuple[str, str] | None:
        """The referral this user is the referee of, if it has yet to pay out.

        Returns `(referral_id, referrer_id)`.
        """
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    select id, referrer_id from referrals
                     where referee_id = %s and status = 'pending'
                    """,
                    (referee_id,),
                )
                row = await cur.fetchone()
        return (row["id"], row["referrer_id"]) if row else None

    async def referral_of(self, referee_id: str) -> str | None:
        """The code this account was referred with, if it was referred at all.

        The code rather than the referrer: the referee is entitled to see what
        they applied and whether it took, and is not entitled to know who is
        being paid for them.
        """
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    "select code from referrals where referee_id = %s", (referee_id,)
                )
                row = await cur.fetchone()
        return row["code"] if row else None

    async def months_earned(self, user_id: str) -> int:
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    "select coalesce(sum(months), 0) as n from rewards where user_id = %s",
                    (user_id,),
                )
                row = await cur.fetchone()
        return int(row["n"]) if row else 0

    async def qualify_referral(
        self, *, referral_id: str, months: int, extend
    ) -> list[str]:
        """Mark a referral qualified and pay both sides, exactly once.

        One transaction, and the whole reason this method is longer than the
        others. `extend` is passed in rather than imported so the arithmetic
        stays in :mod:`app.referrals` and this stays about the transaction.

        The `update ... where status = 'pending'` is the gate: two requests
        arriving together, the second finds nothing to update and pays nobody.
        Returns the ids of the users whose entitlement moved, which is empty
        when this call lost that race.
        """
        paid: list[str] = []
        async with self._pool.connection() as conn:
            async with conn.transaction():
                async with conn.cursor(row_factory=dict_row) as cur:
                    await cur.execute(
                        """
                        update referrals
                           set status = 'qualified', qualified_at = now()
                         where id = %s and status = 'pending'
                        returning referrer_id, referee_id
                        """,
                        (referral_id,),
                    )
                    row = await cur.fetchone()
                    if row is None:
                        return []

                    for side, user_id in (
                        ("referrer", row["referrer_id"]),
                        ("referee", row["referee_id"]),
                    ):
                        # `for update` so a concurrent grant to the same user —
                        # two of their referrals qualifying at once — queues
                        # behind this one instead of reading the same expiry and
                        # both writing a single month onto it.
                        await cur.execute(
                            "select pro_until from users where id = %s for update",
                            (user_id,),
                        )
                        current = await cur.fetchone()
                        until = extend(current["pro_until"] if current else None, months)
                        await cur.execute(
                            "update users set pro_until = %s where id = %s",
                            (until, user_id),
                        )
                        await cur.execute(
                            """
                            insert into rewards (id, user_id, months, source_referral_id, side)
                            values (%s, %s, %s, %s, %s)
                            on conflict (source_referral_id, side) do nothing
                            """,
                            (f"{referral_id}:{side}", user_id, months, referral_id, side),
                        )
                        paid.append(user_id)
        return paid

    async def invites_sent_today(self, user_id: str) -> int:
        """How many invites this account has sent since midnight UTC.

        Counted from the referrals table's sibling — there is no invite log, so
        this counts what the endpoint records: see `record_invite`.
        """
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute(
                    """
                    select count(*) as n from referral_invites
                     where user_id = %s and sent_at > now() - interval '1 day'
                    """,
                    (user_id,),
                )
                row = await cur.fetchone()
        return int(row["n"]) if row else 0

    async def record_invite(self, *, user_id: str, email: str) -> None:
        async with self._pool.connection() as conn:
            await conn.execute(
                "insert into referral_invites (user_id, email) values (%s, %s)",
                (user_id, email),
            )

    async def pro_until(self, user_id: str) -> datetime | None:
        async with self._pool.connection() as conn:
            async with conn.cursor(row_factory=dict_row) as cur:
                await cur.execute("select pro_until from users where id = %s", (user_id,))
                row = await cur.fetchone()
        return row["pro_until"] if row else None
