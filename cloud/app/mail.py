"""Sending the code.

Plain SMTP, deliberately. Every mail provider worth using speaks it — Resend,
Postmark, SES, Mailgun, even Gmail — so the choice of vendor is four
environment variables rather than a client library and a rewrite when the
pricing changes.

Sending blocks, so it runs in a worker thread. It is also allowed to fail
loudly: unlike registering a user, there is no useful outcome without it. A
sign-in that says "check your email" when nothing was sent is worse than one
that says it could not send.
"""

import asyncio
import smtplib
from email.message import EmailMessage

from .codes import TTL_MINUTES
from .config import Settings


class SendFailed(Exception):
    """The mail was not handed to a server that accepted it."""


def _compose(to: str, code: str, sender: str) -> EmailMessage:
    message = EmailMessage()
    message["Subject"] = f"{code} is your Memory OS sign-in code"
    message["From"] = sender
    message["To"] = to
    # The code is in the subject as well as the body. Most people read it off
    # the notification without opening anything.
    message.set_content(
        f"Your sign-in code is {code}\n\n"
        f"It expires in {TTL_MINUTES} minutes and can be used once.\n\n"
        "If you did not ask to sign in, ignore this — nothing has happened to "
        "your account, and nobody can use this code without also having your "
        "email.\n"
    )
    return message


def _send_blocking(settings: Settings, message: EmailMessage) -> None:
    try:
        if settings.smtp_ssl:
            server: smtplib.SMTP = smtplib.SMTP_SSL(settings.smtp_host, settings.smtp_port, timeout=15)
        else:
            server = smtplib.SMTP(settings.smtp_host, settings.smtp_port, timeout=15)
            # Upgrade before authenticating. Without this the password crosses
            # the wire in clear on a plain connection.
            server.starttls()
        with server:
            if settings.smtp_user:
                server.login(settings.smtp_user, settings.smtp_password)
            server.send_message(message)
    except Exception as e:  # noqa: BLE001
        raise SendFailed(str(e)) from e


async def send_code(settings: Settings, to: str, code: str) -> None:
    """Send `code` to `to`, off the event loop."""
    if not settings.smtp_host:
        raise SendFailed("no SMTP server is configured")
    message = _compose(to, code, settings.smtp_from or settings.smtp_user)
    await asyncio.to_thread(_send_blocking, settings, message)
