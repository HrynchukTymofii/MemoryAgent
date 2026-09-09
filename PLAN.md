# Fix: ImmatureSignatureError (iat in the future)

Local clock is seconds behind Google's, so a fresh token looks not-yet-valid.
JWT validation needs a clock-skew leeway.

- `cloud/app/google.py` — 120s leeway on decode
- `cloud/tests/test_google.py` — a token issued slightly in the future is accepted
