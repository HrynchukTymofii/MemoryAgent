"""Referral codes, and the rule that turns one into a month of Pro.

Two decisions are worth stating, because both are the difference between a
referral programme and a way of minting free months.

**A referral pays out on use, not on sign-up.** Anyone can create accounts; not
everyone can dictate two thousand words through one. The qualifying threshold is
the only thing standing between the reward and a script, and it is deliberately
a figure that takes a real person a couple of sessions to reach.

**Both sides are paid from one row, once each.** The `rewards` table has a
unique constraint on `(source_referral_id, side)`, so the grant is an insert
that either lands or collides. A second `POST /progress` cannot pay twice even
if it arrives at the same moment as the first.

Nothing here touches the database. The queries live in :mod:`app.db`; this is
the arithmetic and the vocabulary, which is what makes both testable without a
Postgres to point at.
"""

from __future__ import annotations

import re
import secrets
from datetime import datetime, timedelta, timezone

# The alphabet the *random* half of a code is drawn from, with the ambiguous
# characters removed. These are read off screens, typed into a box, and
# occasionally read aloud down a phone: I/1, O/0 and S/5 are the pairs that cost
# a support message every time they appear.
#
# The name half is not held to this. A name is recognised as a whole and read in
# context, so nobody mistypes the O in their own — and a rule that stripped it
# would turn TYMOFII into TYMF, which is not the person's name and not something
# they would recognise as theirs.
ALPHABET = "ABCDEFGHJKLMNPQRTUVWXY2346789"

# How many random characters follow the name part. Four from this alphabet is
# ~700k combinations per name, which is plenty when a collision only costs a
# retry — and short enough that the whole code fits in a spoken sentence.
SUFFIX = 4

# Longest name fragment kept. A code is a thing to type, not a signature.
NAME_MAX = 10


def slug(display_name: str | None, email: str | None) -> str:
    """The human half of a code: `TYMOFII`, from whatever we know of a person.

    Falls back to the email's local part, then to nothing at all. A code with no
    name in it is still a working code — an account that signed in with a token
    carrying neither field must not be one that cannot refer anybody.
    """
    source = (display_name or "").strip() or (email or "").split("@")[0]
    # ASCII letters only. A name in Cyrillic or one written entirely in accented
    # characters leaves nothing here, and nothing is a valid answer — the code
    # is then four random characters, which still works.
    letters = re.sub(r"[^A-Za-z]", "", source).upper()
    return letters[:NAME_MAX]


def generate(display_name: str | None = None, email: str | None = None) -> str:
    """A candidate code. Uniqueness is the database's job, not this function's.

    `secrets` rather than `random` for the same reason the sign-in codes use it:
    a predictable suffix over a guessable name is a guessable code, and a
    guessable code is somebody else's referral.
    """
    tail = "".join(secrets.choice(ALPHABET) for _ in range(SUFFIX))
    return f"{slug(display_name, email)}{tail}"


def normalise(code: str) -> str:
    """What a user typed, as the database stores it.

    Upper-cased and stripped of punctuation, so a code pasted out of a URL, an
    email signature or a chat message — with a trailing full stop, a zero-width
    space, a hyphen, or wrapped in angle brackets — resolves to the same string
    the sender's link carried.

    Letters and digits both, and not filtered through `ALPHABET`: the name half
    may contain any letter, and stripping the characters that are merely absent
    from the random half would quietly rewrite somebody's code into a shorter
    one that belongs to nobody.
    """
    return "".join(c for c in code.strip().upper() if c.isascii() and c.isalnum())


def looks_like_a_code(code: str) -> bool:
    """Long enough to be one, short enough not to be a paragraph."""
    return SUFFIX <= len(code) <= NAME_MAX + SUFFIX


def invite_link(base_url: str, code: str) -> str:
    """The link that goes in the message someone sends their friend."""
    return f"{base_url.rstrip('/')}/r/{code}"


def has_qualified(words: int, threshold: int) -> bool:
    """Whether a referee has used the app enough for the referral to pay out."""
    return words >= threshold


def may_apply(*, account_created_at: datetime, window_days: int) -> bool:
    """Whether this account is still new enough to have been referred.

    A window, rather than "any time": a referral is a claim that somebody
    brought a new user in, and an account that has been using the app for six
    months was not brought in by the code it applied this morning. Generous
    enough — a month by default — that somebody who was told about the app,
    installed it, and only later got round to asking for the link is not
    punished for the gap.
    """
    if account_created_at.tzinfo is None:
        account_created_at = account_created_at.replace(tzinfo=timezone.utc)
    return datetime.now(timezone.utc) - account_created_at <= timedelta(days=window_days)


def extend(pro_until: datetime | None, months: int) -> datetime:
    """Push an entitlement forward by `months`.

    From whichever is later: now, or the expiry already held. That is what makes
    two rewards stack instead of overwriting each other — granting a month to
    somebody who has three weeks left must leave them with seven, not four.

    A month is 30 days. Calendar months are 28 to 31, and the difference is
    three days of somebody's subscription; a fixed length is the version that
    can be explained in one sentence and audited from the timestamps.
    """
    now = datetime.now(timezone.utc)
    if pro_until is not None and pro_until.tzinfo is None:
        pro_until = pro_until.replace(tzinfo=timezone.utc)
    base = max(now, pro_until) if pro_until is not None else now
    return base + timedelta(days=30 * months)


def is_pro(pro_until: datetime | None) -> bool:
    if pro_until is None:
        return False
    if pro_until.tzinfo is None:
        pro_until = pro_until.replace(tzinfo=timezone.utc)
    return pro_until > datetime.now(timezone.utc)
