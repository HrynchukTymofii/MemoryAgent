"""Settings, read from the environment.

Everything here is a secret or a deployment detail, which is why none of it has
a default worth shipping. The one exception is ``session_ttl_days``: it is a
policy choice, not a credential, and having it in code makes the decision
visible instead of hidden in whatever the environment happened to set.
"""

from functools import lru_cache
from pathlib import Path

from pydantic_settings import BaseSettings, SettingsConfigDict

# The repository root, found from this file rather than from the working
# directory. `env_file=".env"` is resolved relative to wherever the process was
# started, so running `uvicorn` from `cloud/` looked for `cloud/.env` and found
# nothing — reported as three missing fields rather than as a missing file.
_ROOT = Path(__file__).resolve().parents[2]


class Settings(BaseSettings):
    # Both, in order: the repository root is where the desktop app's `.env`
    # already lives, and a `cloud/.env` is what a deployment that only ships
    # this service would have.
    model_config = SettingsConfigDict(
        env_file=(_ROOT / ".env", _ROOT / "cloud" / ".env"),
        env_file_encoding="utf-8",
        extra="ignore",
    )

    # The direct Postgres connection string. Lives here and only here: this is
    # the credential the desktop app must never hold, and the whole reason this
    # service exists.
    database_url: str

    # The audience every Google identity token must name. Tokens minted for
    # some other application are rejected, which is what stops anyone with a
    # valid Google login from presenting one here.
    google_client_id: str

    # Signs the session tokens this service issues. Rotating it signs everyone
    # out, which is the intended emergency behaviour.
    session_secret: str

    # Long enough that a person is not asked to sign in again on a laptop they
    # use weekly, short enough that a leaked token is not permanent.
    session_ttl_days: int = 30

    # Mail, for email sign-in. Empty host disables it and the endpoints say so
    # rather than failing obscurely.
    smtp_host: str = ""
    smtp_port: int = 587
    smtp_user: str = ""
    smtp_password: str = ""
    smtp_from: str = ""
    # Port 465 is implicit TLS; 587 upgrades with STARTTLS. Both are encrypted,
    # and getting this wrong looks like a hang rather than an error.
    smtp_ssl: bool = False

    # ------------------------------------------------------------ referrals

    # Where an invite link sends someone who is not a user yet. Empty until
    # there is a site to send them to; `GET /r/{code}` still records the click
    # and says plainly that there is nowhere to go, rather than redirecting to
    # a page that does not exist.
    download_url: str = ""

    # Where the desktop app reaches this service. Only used to build the invite
    # link that is shown and shared, which is why it is a separate value from
    # anything internal: the link has to work from a stranger's phone.
    public_url: str = "https://api.memoryos.app"

    # What a referee has to actually do before either side is paid. Anyone can
    # create accounts; not everyone can dictate two thousand words through one.
    referral_qualify_words: int = 2_000

    # How long after signing up an account may still apply a code. A referral
    # is a claim that somebody brought a new user in, and an account six months
    # old was not brought in by the code it applied this morning.
    referral_window_days: int = 30

    # Months of Pro, per side, per qualifying referral.
    referral_months: int = 1

    # Invites one account may send in a day. High enough that nobody legitimate
    # notices, low enough that this is not a mail cannon pointed at strangers.
    referral_invites_per_day: int = 20


@lru_cache
def settings() -> Settings:
    """Read once, at startup. Cached so a request never touches the environment.

    Worth knowing while developing: `uvicorn --reload` watches `.py` files, not
    `.env`. Editing credentials leaves the running process holding the old ones
    and answering 401 with a configuration that has since been fixed on disk —
    restart the server after changing `.env`.
    """
    return Settings()  # type: ignore[call-arg]
