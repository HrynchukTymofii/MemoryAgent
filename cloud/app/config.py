"""Settings, read from the environment.

Everything here is a secret or a deployment detail, which is why none of it has
a default worth shipping. The one exception is ``session_ttl_days``: it is a
policy choice, not a credential, and having it in code makes the decision
visible instead of hidden in whatever the environment happened to set.
"""

from functools import lru_cache

from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=".env", extra="ignore")

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


@lru_cache
def settings() -> Settings:
    """Read once. Cached so a request never touches the environment."""
    return Settings()  # type: ignore[call-arg]
