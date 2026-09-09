"""The session tokens this service issues.

Google's identity token proves who someone is, once, for an hour. It is the
wrong thing to keep presenting: it expires quickly, refreshing it means going
back to Google, and it is scoped to Google rather than to us. So it is
exchanged exactly once, at sign-in, for a token of our own.

Ours is a plain signed JWT with a `sub` and an expiry. It carries no
permissions and no profile — only an identifier — so a leaked token grants
whatever that user could do and nothing more, and revoking it is a matter of
rotating one secret.
"""

from datetime import datetime, timedelta, timezone

import jwt

ALGORITHM = "HS256"
ISSUER = "memory-os"


class InvalidSession(Exception):
    """The token is missing, malformed, expired or not ours."""


def issue(sub: str, *, secret: str, ttl_days: int) -> tuple[str, datetime]:
    """Mint a session token for `sub`, and say when it stops working.

    The expiry is returned alongside rather than left for the client to decode,
    so the desktop app can renew ahead of time without parsing a JWT.
    """
    now = datetime.now(timezone.utc)
    expires = now + timedelta(days=ttl_days)
    token = jwt.encode(
        {"sub": sub, "iss": ISSUER, "iat": now, "exp": expires},
        secret,
        algorithm=ALGORITHM,
    )
    return token, expires


def subject(token: str, *, secret: str) -> str:
    """Return the user id a token is for, or raise.

    `require` is explicit: a token with no expiry would otherwise verify
    happily and last forever, which is the one property a session token must
    never have.
    """
    try:
        claims = jwt.decode(
            token,
            secret,
            algorithms=[ALGORITHM],
            issuer=ISSUER,
            options={"require": ["exp", "sub", "iss"]},
        )
    except Exception as e:  # noqa: BLE001
        raise InvalidSession(str(e)) from e
    return claims["sub"]
