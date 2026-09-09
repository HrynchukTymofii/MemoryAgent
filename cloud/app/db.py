"""The connection pool, and the two queries there are so far.

SQL is written here rather than through an ORM. There are two statements, the
schema is in `cloud/migrations/` where it can be read, and an ORM would add a
layer of indirection over queries that are already shorter than their own
mapping would be.
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

    @classmethod
    def of(cls, row: dict[str, Any]) -> "User":
        return cls(
            id=row["id"],
            email=row["email"],
            display_name=row["display_name"],
            created_at=row["created_at"],
            last_seen_at=row["last_seen_at"],
        )


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
