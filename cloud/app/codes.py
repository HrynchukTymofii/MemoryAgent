"""One-time sign-in codes.

Six digits, five minutes, five attempts. Each of those numbers is a trade
between a person retyping something from their phone and a script guessing a
million possibilities, and the attempt limit is the one that actually matters:
six digits is only strong because guessing stops.

Codes are stored as SHA-256 hashes. They are short-lived and low-entropy, so
this is not password hashing and a slow KDF would buy nothing — what it buys is
that a leaked snapshot of the table is not a list of working codes.
"""

import hashlib
import hmac
import secrets

LENGTH = 6
TTL_MINUTES = 5
MAX_ATTEMPTS = 5


def generate() -> str:
    """A code, from the OS random source.

    `secrets`, not `random`: the latter is a Mersenne twister whose output is
    predictable from a handful of previous values, which for a sign-in code
    means predictable full stop.
    """
    return "".join(secrets.choice("0123456789") for _ in range(LENGTH))


def hash_code(code: str) -> str:
    return hashlib.sha256(code.encode()).hexdigest()


def matches(code: str, code_hash: str) -> bool:
    """Compare in constant time.

    `==` on hex strings returns as soon as two characters differ, and the time
    that takes is measurable over a network. Overkill for six digits with an
    attempt limit, and still the correct habit.
    """
    return hmac.compare_digest(hash_code(code), code_hash)


def normalise(email: str) -> str:
    """Lower-cased and trimmed, so `A@B.com ` and `a@b.com` are one account.

    Only the case and whitespace. The local part of an address is
    case-sensitive by the letter of the spec and nobody implements it that way;
    stripping dots or plus-addressing, which some providers treat as
    equivalent, would merge addresses their owners consider separate.
    """
    return email.strip().lower()


def looks_like_an_address(email: str) -> bool:
    """A shape check, not validation.

    The only real proof an address exists is that a code sent to it comes back,
    which is the entire mechanism here. This exists to catch a typo before
    spending a send on it.
    """
    if len(email) > 254 or email.count("@") != 1:
        return False
    local, _, domain = email.partition("@")
    return bool(local) and "." in domain and not domain.startswith(".") and not domain.endswith(".")
