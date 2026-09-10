"""The API.

Deliberately no more than the desktop app needs today:

    POST /v1/auth/google       exchange a Google identity token for a session
    POST /v1/auth/email/*      the same, by one-time code
    GET  /v1/me                who the caller is
    GET  /v1/referrals/me      the caller's code, invites and rewards
    POST /v1/referrals/apply   be referred by somebody
    POST /v1/referrals/invite  send invites by mail
    POST /v1/referrals/progress  report use, which is what pays a referral out
    GET  /r/{code}             the link in an invite
    GET  /health               is this process alive and can it reach Postgres

This service exists for one structural reason: it is the only place that may
hold a database password. The desktop app ships to everyone and can keep no
secret, so it holds a session token scoped to one user and talks to this. Every
credential that must stay private — the Postgres URL, the session signing key,
and later the Apple and GitHub client secrets and whatever sends email — lives
here, on a machine we control.
"""

import logging
from contextlib import asynccontextmanager
from datetime import datetime

import uuid

from fastapi import Depends, FastAPI, Header, HTTPException, status
from fastapi.responses import RedirectResponse
from pydantic import BaseModel

from . import codes, google, mail, referrals, tokens
from .config import settings
from .db import Database, User

log = logging.getLogger("memos")

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
        # To the log, not to the caller. Which check failed is useful to
        # someone probing this endpoint and useless to a person who simply
        # needs to sign in again — but it has to be written down *somewhere*,
        # or the 401 is undiagnosable by the person running the service.
        log.warning("rejected an identity token: %s", e)
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "invalid identity token") from e

    account = await store.upsert_user(sub=user.sub, email=user.email, display_name=user.name)
    token, expires = tokens.issue(
        user.sub, secret=config.session_secret, ttl_days=config.session_ttl_days
    )
    return SignInResponse(token=token, expires_at=expires, account=Account.of(account))


# --------------------------------------------------------------- email sign-in


class EmailStartRequest(BaseModel):
    email: str


class EmailVerifyRequest(BaseModel):
    email: str
    code: str


@app.post("/v1/auth/email/start", status_code=status.HTTP_204_NO_CONTENT)
async def email_start(body: EmailStartRequest, store: Database = Depends(database)) -> None:
    """Send a one-time code to an address.

    Returns 204 whether or not the address belongs to anyone. Answering
    differently would turn this endpoint into a way to ask "does this person
    have an account here", which is not a question a stranger gets to ask.
    Genuine failures — a mail server that would not accept the message — are
    still errors, because the user is staring at a screen that would otherwise
    claim something was sent.
    """
    config = settings()
    if not config.smtp_host:
        raise HTTPException(
            status.HTTP_501_NOT_IMPLEMENTED, "email sign-in is not configured"
        )

    email = codes.normalise(body.email)
    if not codes.looks_like_an_address(email):
        raise HTTPException(status.HTTP_400_BAD_REQUEST, "that does not look like an address")

    code = codes.generate()
    await store.store_code(
        email=email, code_hash=codes.hash_code(code), ttl_minutes=codes.TTL_MINUTES
    )
    try:
        await mail.send_code(config, email, code)
    except mail.SendFailed as e:
        raise HTTPException(status.HTTP_502_BAD_GATEWAY, "could not send the code") from e


@app.post("/v1/auth/email/verify", response_model=SignInResponse)
async def email_verify(
    body: EmailVerifyRequest, store: Database = Depends(database)
) -> SignInResponse:
    """Exchange a correct code for a session.

    Every failure below is the same message. Distinguishing "no code was
    requested" from "the code is wrong" from "you have guessed too many times"
    tells someone probing the endpoint exactly where they are, and tells a
    legitimate user nothing they can act on beyond asking for a new code.
    """
    email = codes.normalise(body.email)
    stored = await store.take_code(email=email, max_attempts=codes.MAX_ATTEMPTS)
    if stored is None or not codes.matches(body.code.strip(), stored):
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "that code is not valid")

    # Correct: spend it, so it cannot be replayed.
    await store.clear_code(email)

    # An email account has no `sub` from a provider, so one is derived from the
    # address. Stable, and namespaced so it can never collide with a Google
    # subject — the same person signing in both ways is two accounts today, and
    # merging them is a deliberate feature rather than an accident of key
    # collision.
    existing = await store.user_by_email(email)
    sub = existing.id if existing else f"email:{email}"

    account = await store.upsert_user(sub=sub, email=email, display_name=None)
    config = settings()
    token, expires = tokens.issue(
        sub, secret=config.session_secret, ttl_days=config.session_ttl_days
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


# ------------------------------------------------------------------ referrals


class ReferralRow(BaseModel):
    """One person you brought in.

    The address is masked. The referrer is owed a count and a status — enough to
    see that their invite worked — and is not owed somebody else's email, which
    they may only have typed into the invite box and never had a right to keep.
    """

    who: str
    status: str
    created_at: datetime
    qualified_at: datetime | None


class ReferralStatus(BaseModel):
    code: str
    link: str
    #: How many words the referee has to dictate before either side is paid.
    qualify_words: int
    months_per_referral: int
    referrals: list[ReferralRow]
    #: Qualified referrals only. A pending one has earned nobody anything yet.
    months_earned: int
    pro_until: datetime | None
    #: Whether this account may still be referred by somebody else.
    can_apply: bool
    #: The code this account was referred with, once it has applied one.
    applied_code: str | None


def _mask(email: str | None) -> str:
    """`tim.20049090@gmail.com` becomes `t…@gmail.com`."""
    if not email or "@" not in email:
        return "someone"
    local, _, domain = email.partition("@")
    return f"{local[:1]}…@{domain}"


async def _code_for(user: User, store: Database) -> str:
    """This user's code, minting one the first time they ask.

    Retried on collision rather than pre-checked: the check and the insert would
    be two statements with a gap between them, and the gap is exactly where two
    people get the same code. Ten attempts over ~700k combinations per name is
    not a limit anyone reaches; it is there so a misconfigured alphabet fails
    loudly instead of looping.
    """
    existing = await store.referral_code(user.id)
    if existing:
        return existing
    for _ in range(10):
        candidate = referrals.generate(user.display_name, user.email)
        claimed = await store.claim_referral_code(user_id=user.id, code=candidate)
        if claimed:
            return claimed
    raise HTTPException(status.HTTP_503_SERVICE_UNAVAILABLE, "could not allocate a code")


@app.get("/v1/referrals/me", response_model=ReferralStatus)
async def my_referrals(
    user: User = Depends(caller), store: Database = Depends(database)
) -> ReferralStatus:
    config = settings()
    code = await _code_for(user, store)
    mine = await store.referrals_by(user.id)
    was_referred = await store.referral_of(user.id)

    return ReferralStatus(
        code=code,
        link=referrals.invite_link(config.public_url, code),
        qualify_words=config.referral_qualify_words,
        months_per_referral=config.referral_months,
        referrals=[
            ReferralRow(
                who=_mask(r.referee_email),
                status=r.status,
                created_at=r.created_at,
                qualified_at=r.qualified_at,
            )
            for r in mine
        ],
        months_earned=await store.months_earned(user.id),
        pro_until=await store.pro_until(user.id),
        can_apply=was_referred is None
        and referrals.may_apply(
            account_created_at=user.created_at, window_days=config.referral_window_days
        ),
        applied_code=was_referred,
    )


class ApplyRequest(BaseModel):
    code: str


@app.post("/v1/referrals/apply", response_model=ReferralStatus)
async def apply_referral(
    body: ApplyRequest, user: User = Depends(caller), store: Database = Depends(database)
) -> ReferralStatus:
    """Be referred by somebody.

    Four refusals, and each one is a rule rather than a validation: it is not a
    code, it is your own code, you have already been referred, or your account
    is too old to have been brought in by anyone. The messages differ because
    every one of them is something the user can act on — unlike the sign-in
    endpoints, where telling the caller which check failed tells an attacker
    where they are.
    """
    config = settings()
    code = referrals.normalise(body.code)
    if not referrals.looks_like_a_code(code):
        raise HTTPException(status.HTTP_400_BAD_REQUEST, "that does not look like a code")

    referrer = await store.user_by_referral_code(code)
    if referrer is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "no such code")
    if referrer.id == user.id:
        raise HTTPException(status.HTTP_400_BAD_REQUEST, "that is your own code")
    if not referrals.may_apply(
        account_created_at=user.created_at, window_days=config.referral_window_days
    ):
        raise HTTPException(
            status.HTTP_409_CONFLICT,
            f"a code can only be applied in the first {config.referral_window_days} days",
        )

    recorded = await store.record_referral(
        referral_id=str(uuid.uuid4()),
        referrer_id=referrer.id,
        referee_id=user.id,
        code=code,
    )
    if not recorded:
        raise HTTPException(status.HTTP_409_CONFLICT, "this account has already been referred")

    log.info("referral applied: %s referred by %s", user.id, referrer.id)
    return await my_referrals(user=user, store=store)


class ProgressRequest(BaseModel):
    """Lifetime words dictated, as the client counts them."""

    words: int


class ProgressResponse(BaseModel):
    qualified: bool
    pro_until: datetime | None


@app.post("/v1/referrals/progress", response_model=ProgressResponse)
async def report_progress(
    body: ProgressRequest, user: User = Depends(caller), store: Database = Depends(database)
) -> ProgressResponse:
    """Tell us how much this account has used the app, and pay out if it is
    enough.

    The figure is reported by the client, which means it can be lied about by
    anyone willing to modify the binary. That is accepted: the alternative is
    shipping every transcript to a server that has no other reason to see one,
    and the thing being protected is a month of a product that has no price yet.
    The constraints that actually matter — one referral per account, one payout
    per side — are in the database and are not client-side at all.
    """
    config = settings()
    pending = await store.pending_referral_of(user.id)
    if pending is None or not referrals.has_qualified(
        body.words, config.referral_qualify_words
    ):
        return ProgressResponse(
            qualified=False, pro_until=await store.pro_until(user.id)
        )

    referral_id, _ = pending
    paid = await store.qualify_referral(
        referral_id=referral_id, months=config.referral_months, extend=referrals.extend
    )
    if paid:
        log.info("referral %s qualified; paid %s", referral_id, ", ".join(paid))
    return ProgressResponse(qualified=bool(paid), pro_until=await store.pro_until(user.id))


class InviteRequest(BaseModel):
    emails: list[str]


class InviteResponse(BaseModel):
    sent: list[str]
    failed: list[str]


@app.post("/v1/referrals/invite", response_model=InviteResponse)
async def send_invites(
    body: InviteRequest, user: User = Depends(caller), store: Database = Depends(database)
) -> InviteResponse:
    """Mail an invite to each address.

    Rate-limited per account per day. This endpoint sends mail, unsolicited, to
    addresses typed by somebody else — the limit is not about our costs, it is
    the difference between a referral feature and a mail cannon.
    """
    config = settings()
    if not config.smtp_host:
        raise HTTPException(status.HTTP_501_NOT_IMPLEMENTED, "mail is not configured")

    wanted = [codes.normalise(e) for e in body.emails]
    wanted = [e for e in wanted if codes.looks_like_an_address(e)]
    if not wanted:
        raise HTTPException(status.HTTP_400_BAD_REQUEST, "no usable addresses")

    already = await store.invites_sent_today(user.id)
    room = max(0, config.referral_invites_per_day - already)
    if room == 0:
        raise HTTPException(
            status.HTTP_429_TOO_MANY_REQUESTS,
            f"that is {config.referral_invites_per_day} invites today; try again tomorrow",
        )

    code = await _code_for(user, store)
    link = referrals.invite_link(config.public_url, code)
    inviter = user.display_name or (user.email or "").split("@")[0] or "A friend"

    sent: list[str] = []
    failed: list[str] = []
    for address in wanted[:room]:
        try:
            await mail.send_invite(
                config, address, inviter=inviter, link=link, months=config.referral_months
            )
        except mail.SendFailed as e:
            # One bad address must not lose the others. The caller is told
            # which went and which did not, and can retry only the failures.
            log.warning("invite to %s failed: %s", _mask(address), e)
            failed.append(address)
            continue
        await store.record_invite(user_id=user.id, email=address)
        sent.append(address)

    return InviteResponse(sent=sent, failed=failed)


@app.get("/r/{code}")
async def follow_invite(code: str, store: Database = Depends(database)) -> RedirectResponse:
    """The link in an invite.

    Redirects to wherever the app is downloaded. The code is not applied here
    and could not be: nobody is signed in yet, and the person clicking may not
    have an account for another ten minutes. It travels in the query string so
    the download page can carry it into the app, and the app applies it once
    there is somebody to apply it to.
    """
    config = settings()
    normalised = referrals.normalise(code)
    if not config.download_url:
        # Honest rather than a redirect to nowhere. Somebody is holding a link
        # a user sent them, and a 404 at least says the link is not the problem.
        raise HTTPException(
            status.HTTP_503_SERVICE_UNAVAILABLE, "there is no download page configured yet"
        )
    separator = "&" if "?" in config.download_url else "?"
    return RedirectResponse(f"{config.download_url}{separator}r={normalised}")


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
