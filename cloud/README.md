# Cloud API

The one place that may hold a database password.

The desktop app ships to everyone and can keep no secret, so it holds a session
token scoped to a single user and talks to this. Every credential that must stay
private lives here: the Postgres URL, the session signing key, and — when Apple,
GitHub and email sign-in arrive — their secrets and whatever sends mail. That is
the whole reason this service exists, and why those three could not be built
without it.

```
desktop --id_token--> API --verifies--> Google
                       |
                       +--SQL--> Postgres
                       |
        <--session token--
```

## Endpoints

| | |
|---|---|
| `POST /v1/auth/google` | exchange a Google identity token for a session |
| `GET /v1/me` | who the caller is |
| `POST /v1/dictation/shape` | format a dictated transcript |
| `GET /health` | alive, and able to reach Postgres |

`/v1/dictation/shape` is the odd one out and the only endpoint here that takes
no session token. It reads no database, identifies nobody, and returns nothing
the caller did not send — a formatting of their own sentence. Requiring a
sign-in would mean dictation stopped working whenever Postgres did.

That reasoning stops holding the moment this service is public, where an
unauthenticated endpoint that runs a model on demand is somebody else's free
compute. **Put it behind `caller` before deploying.**

It needs `models/llm/router.gguf` (Qwen3-0.6B, `scriptsetch-models.ps1
router`) and `llama-cpp-python`. Without either, the service starts, logs why,
and answers that one endpoint with a 503 — which the desktop app already falls
back from by typing the unformatted words.

## Running it

```
cd cloud
python -m venv .venv && .venv\Scripts\activate
pip install -r requirements.txt
uvicorn app.main:app --reload --port 8000
```

It reads `.env` at the repository root:

```
DATABASE_URL=postgresql://...          # Neon, direct. Never leaves this service
GOOGLE_CLIENT_ID=...                   # the same client the desktop app uses
SESSION_SECRET=<32+ random bytes>      # rotating it signs everyone out
```

`GOOGLE_CLIENT_ID` must match `MEMOS_GOOGLE_CLIENT_ID`. That check is what stops
a token minted for some other application being replayed here — without it, any
valid Google login in the world would be accepted as one of ours.

Apply the schema first:

```
.\scripts\migrate-cloud.ps1
```

## When sign-in returns 401

The server logs the real reason — audience mismatch, expired, bad signature.
Look there first.

`--reload` watches `.py` files, not `.env`. After editing credentials, restart
the server: the running process keeps the settings it read at startup and will
keep rejecting tokens against a configuration you have already fixed.

## Tests

```
cd cloud && python -m pytest
```

They cover the two things worth covering: that a valid token yields the right
user, and that every way of forging one fails — wrong audience, wrong issuer,
wrong signature, no signature, expired, no subject. Those are the security
boundary of the service, and they run without a database or a network.
