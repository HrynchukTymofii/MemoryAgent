"""Identity-token verification — the security boundary of the service.

Signed with a throwaway RSA key and a stub key client, so these exercise the
real code path rather than a mock of it.
"""

import time

import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import rsa

from app import google

AUDIENCE = "our-client-id.apps.googleusercontent.com"
KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)


class StubKeys:
    """Stands in for PyJWKClient, returning a key we control."""

    def __init__(self, key=None):
        self._key = key if key is not None else KEY.public_key()

    def get_signing_key_from_jwt(self, _token):
        return type("K", (), {"key": self._key})()


def token(**overrides):
    claims = {
        "sub": "google-user-1",
        "iss": "https://accounts.google.com",
        "aud": AUDIENCE,
        "email": "someone@example.com",
        "email_verified": True,
        "name": "Someone",
        "iat": int(time.time()),
        "exp": int(time.time()) + 3600,
    }
    claims.update(overrides)
    for k, v in list(claims.items()):
        if v is None:
            del claims[k]
    return jwt.encode(claims, KEY, algorithm="RS256")


def verify(t):
    return google.verify(t, audience=AUDIENCE, _client=StubKeys())


def test_a_valid_token_yields_the_user_it_describes():
    user = verify(token())
    assert user.sub == "google-user-1"
    assert user.email == "someone@example.com"
    assert user.name == "Someone"
    assert user.email_verified is True


def test_a_token_for_another_application_is_rejected():
    """Without the audience check, any valid Google token would work here —
    including one minted for an unrelated app by someone else entirely."""
    with pytest.raises(google.InvalidToken):
        verify(token(aud="some-other-app.apps.googleusercontent.com"))


def test_a_token_from_another_issuer_is_rejected():
    with pytest.raises(google.InvalidToken):
        verify(token(iss="https://evil.example.com"))


def test_both_spellings_of_googles_issuer_are_accepted():
    """Google uses both, and tokens appear in the wild with either."""
    assert verify(token(iss="accounts.google.com")).sub == "google-user-1"


def test_an_expired_token_is_rejected():
    # Comfortably past the clock-skew leeway. Ten seconds stale is inside it,
    # deliberately — that is drift, not an expired token.
    with pytest.raises(google.InvalidToken):
        verify(token(exp=int(time.time()) - google.CLOCK_SKEW_SECONDS - 60))


def test_a_token_signed_by_someone_else_is_rejected():
    other = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    forged = jwt.encode(
        {
            "sub": "google-user-1",
            "iss": "https://accounts.google.com",
            "aud": AUDIENCE,
            "iat": int(time.time()),
            "exp": int(time.time()) + 3600,
        },
        other,
        algorithm="RS256",
    )
    with pytest.raises(google.InvalidToken):
        verify(forged)


def test_an_unsigned_token_is_rejected():
    forged = jwt.encode(
        {
            "sub": "google-user-1",
            "iss": "https://accounts.google.com",
            "aud": AUDIENCE,
            "exp": int(time.time()) + 3600,
        },
        key="",
        algorithm="none",
    )
    with pytest.raises(google.InvalidToken):
        verify(forged)


def test_an_unverified_email_is_recorded_rather_than_refused():
    """Worth having, and worth knowing about — but not proof of reaching them."""
    user = verify(token(email_verified=False))
    assert user.email == "someone@example.com"
    assert user.email_verified is False


def test_a_token_with_no_subject_is_rejected():
    with pytest.raises(google.InvalidToken):
        verify(token(sub=None))


def test_a_token_issued_seconds_in_the_future_is_accepted():
    """Clock skew, not an attack.

    `iat` is stamped by Google and checked against this machine's clock. A
    workstation a few seconds slow makes a token issued moments ago look like
    it is not yet valid, and sign-in fails with nothing actually wrong.
    """
    user = verify(token(iat=int(time.time()) + 30))
    assert user.sub == "google-user-1"


def test_the_leeway_does_not_stretch_to_a_genuinely_future_token():
    with pytest.raises(google.InvalidToken):
        verify(token(iat=int(time.time()) + google.CLOCK_SKEW_SECONDS + 300))


def test_a_token_expired_beyond_the_leeway_is_still_rejected():
    with pytest.raises(google.InvalidToken):
        verify(token(exp=int(time.time()) - google.CLOCK_SKEW_SECONDS - 300))
