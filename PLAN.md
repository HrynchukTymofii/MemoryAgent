# Fix: 401 from /v1/auth/google says nothing

The reason is discarded, so the failure is undiagnosable.

- `cloud/app/main.py` — log the real verification failure
- `cloud/app/google.py` — carry the specific reason on the exception
