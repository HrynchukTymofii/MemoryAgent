"""Verifying a Google identity token.

The desktop app performs the OAuth flow itself and arrives here holding an
``id_token``. That token is a signed assertion from Google about who the user
is — but only if it is actually checked, and checking it is the entire security
boundary of this service. An unverified token is a text field the caller
controls, and treating one as an identity would let anybody be anybody.

Three things must be true, and all three are checked:

- the signature is Google's, against a key from Google's published key set
- the audience is *our* client id, so a token minted for another application
  cannot be replayed here
- the issuer is Google, and the token has not expired

``PyJWKClient`` caches the key set and refetches when it sees an unknown key
id, so key rotation needs no deployment.
"""

from dataclasses import dataclass

import jwt
from jwt import PyJWKClient

# Google's published signing keys, and the two issuer spellings it uses. Both
# are valid and tokens appear with either, so both are accepted.
JWKS_URL = "https://www.googleapis.com/oauth2/v3/certs"
ISSUERS = ("https://accounts.google.com", "accounts.google.com")

_jwks = PyJWKClient(JWKS_URL, cache_keys=True)


class InvalidToken(Exception):
    """The token is not a valid assertion about anybody."""


@dataclass(frozen=True)
class GoogleUser:
    """What Google is willing to say about the person who signed in."""

    sub: str
    email: str | None
    name: str | None
    email_verified: bool


def verify(id_token: str, *, audience: str, _client: PyJWKClient | None = None) -> GoogleUser:
    """Check a token and return who it describes.

    Raises :class:`InvalidToken` for every failure, deliberately without
    detail. The caller is a client that has just failed to authenticate; which
    specific check failed is useful to an attacker probing the endpoint and
    useless to a legitimate user, so it goes to the log rather than the wire.
    """
    client = _client or _jwks
    try:
        key = client.get_signing_key_from_jwt(id_token).key
        claims = jwt.decode(
            id_token,
            key,
            algorithms=["RS256"],
            audience=audience,
            # Checked explicitly below: PyJWT accepts only a single issuer
            # string, and Google uses two spellings.
            options={"verify_iss": False, "require": ["exp", "iat", "sub", "aud"]},
        )
    except Exception as e:  # noqa: BLE001 - every failure is the same answer
        # The class name matters as much as the message: PyJWT's
        # `InvalidAudienceError` and `ExpiredSignatureError` are two very
        # different setup problems and their messages alone do not distinguish
        # them clearly.
        raise InvalidToken(f"{type(e).__name__}: {e}") from e

    if claims.get("iss") not in ISSUERS:
        raise InvalidToken(f"unexpected issuer {claims.get('iss')!r}")

    return GoogleUser(
        sub=claims["sub"],
        email=claims.get("email"),
        name=claims.get("name"),
        # Google sets this false for addresses it has not confirmed. Stored
        # rather than enforced: an unverified address is still worth having,
        # it just must not be treated as proof of reaching that person.
        email_verified=bool(claims.get("email_verified", False)),
    )
