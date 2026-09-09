"""The API.

Three endpoints, and deliberately no more than the desktop app needs today:

    POST /v1/auth/google   exchange a Google identity token for a session
    GET  /v1/me            who the caller is
    GET  /health           is this process alive and can it reach Postgres

This service exists for one structural reason: it is the only place that may
hold a database password. The desktop app ships to everyone and can keep no
secret, so it holds a session token scoped to one user and talks to this. Every
credential that must stay private — the Postgres URL, the session signing key,
and later the Apple and GitHub client secrets and whatever sends email — lives
here, on a machine we control.
"""

from contextlib import asynccontextmanager
from datetime import datetime

from fastapi import Depends, FastAPI, Header, HTTPException, status
from pydantic import BaseModel

from . import google, tokens
from .config import settings
from .db import Database, User

db: Database | None = None


@asynccontextmanager
async def lifespan(app: FastAPI):
    """Open the pool before the first request and close it after the last.

    Opened here rather than at import so the module can be imported — by tests,
    by tooling — without a reachable database.
    """
    global db
    db = Database(settings().database_url)
    await db.open()
    try:
        yield
    finally:
        await db.close()
        db = None


app = FastAPI(title="Memory OS", version="0.1.0", lifespan=lifespan)


def database() -> Database:
    if db is None:
        raise HTTPException(status.HTTP_503_SERVICE_UNAVAILABLE, "not ready")
    return db


class SignInRequest(BaseModel):
    id_token: str


class Account(BaseModel):
    """What the client is told about itself. No tokens beyond its own session."""

    id: str
    email: str | None
    display_name: str | None
    created_at: datetime
    last_seen_at: datetime

    @classmethod
    def of(cls, u: User) -> "Account":
        return cls(
            id=u.id,
            email=u.email,
            display_name=u.display_name,
            created_at=u.created_at,
            last_seen_at=u.last_seen_at,
        )


class SignInResponse(BaseModel):
    token: str
    expires_at: datetime
    account: Account


@app.post("/v1/auth/google", response_model=SignInResponse)
async def sign_in_with_google(
    body: SignInRequest, store: Database = Depends(database)
) -> SignInResponse:
    """Turn a Google identity token into a session with us.

    The verification in :mod:`google` is the security boundary of the whole
    service — everything after it trusts `sub`, so nothing before it may.
    """
    config = settings()
    try:
        user = google.verify(body.id_token, audience=config.google_client_id)
    except google.InvalidToken as e:
        # The specific failure goes to the log, not to the caller: which check
        # failed is useful to someone probing this endpoint and useless to a
        # person who simply needs to sign in again.
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "invalid identity token") from e

    account = await store.upsert_user(sub=user.sub, email=user.email, display_name=user.name)
    token, expires = tokens.issue(
        user.sub, secret=config.session_secret, ttl_days=config.session_ttl_days
    )
    return SignInResponse(token=token, expires_at=expires, account=Account.of(account))


async def caller(
    authorization: str | None = Header(default=None),
    store: Database = Depends(database),
) -> User:
    """Resolve `Authorization: Bearer <session token>` to a user."""
    scheme, _, token = (authorization or "").partition(" ")
    if scheme.lower() != "bearer" or not token:
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "missing bearer token")
    try:
        sub = tokens.subject(token, secret=settings().session_secret)
    except tokens.InvalidSession as e:
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "invalid session") from e

    user = await store.user(sub)
    if user is None:
        # A validly signed token for a user who no longer exists — deleted
        # account, or a database restored from before they signed up. The token
        # is genuine and still must not work.
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "unknown user")
    return user


@app.get("/v1/me", response_model=Account)
async def me(user: User = Depends(caller)) -> Account:
    return Account.of(user)


@app.get("/health")
async def health() -> dict[str, str]:
    """Alive, and able to reach Postgres.

    The query is the point. A health check that only proves the process is
    running is one that reports healthy through an outage of the only thing it
    depends on.
    """
    store = database()
    await store.user("__health__")
    return {"status": "ok"}
