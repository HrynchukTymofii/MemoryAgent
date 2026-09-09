"""Session tokens: the properties that must hold, not the ones that are easy."""

import time

import jwt
import pytest

from app import tokens

SECRET = "test-secret"


def test_a_token_round_trips_to_the_user_it_was_issued_for():
    token, expires = tokens.issue("user-1", secret=SECRET, ttl_days=30)
    assert tokens.subject(token, secret=SECRET) == "user-1"
    assert expires > _now()


def test_a_token_signed_with_another_secret_is_rejected():
    token, _ = tokens.issue("user-1", secret="someone-elses-secret", ttl_days=30)
    with pytest.raises(tokens.InvalidSession):
        tokens.subject(token, secret=SECRET)


def test_an_expired_token_is_rejected():
    token, _ = tokens.issue("user-1", secret=SECRET, ttl_days=-1)
    with pytest.raises(tokens.InvalidSession):
        tokens.subject(token, secret=SECRET)


def test_a_token_with_no_expiry_is_rejected():
    """The one property a session token must never have is lasting forever."""
    forged = jwt.encode({"sub": "user-1", "iss": tokens.ISSUER}, SECRET, algorithm="HS256")
    with pytest.raises(tokens.InvalidSession):
        tokens.subject(forged, secret=SECRET)


def test_an_unsigned_token_is_rejected():
    """`alg: none` is the oldest JWT attack there is."""
    forged = jwt.encode(
        {"sub": "user-1", "iss": tokens.ISSUER, "exp": time.time() + 3600},
        key="",
        algorithm="none",
    )
    with pytest.raises(tokens.InvalidSession):
        tokens.subject(forged, secret=SECRET)


def test_a_token_from_another_issuer_is_rejected():
    forged = jwt.encode(
        {"sub": "user-1", "iss": "somebody-else", "exp": time.time() + 3600},
        SECRET,
        algorithm="HS256",
    )
    with pytest.raises(tokens.InvalidSession):
        tokens.subject(forged, secret=SECRET)


def test_garbage_is_rejected_rather_than_crashing():
    for junk in ["", "not.a.token", "a.b.c", "Bearer x"]:
        with pytest.raises(tokens.InvalidSession):
            tokens.subject(junk, secret=SECRET)


def _now():
    from datetime import datetime, timezone

    return datetime.now(timezone.utc)
