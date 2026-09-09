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


@lru_cache
def settings() -> Settings:
    """Read once. Cached so a request never touches the environment."""
    return Settings()  # type: ignore[call-arg]
