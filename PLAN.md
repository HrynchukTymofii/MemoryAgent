# Fix: API cannot find .env

pydantic-settings looks relative to the working directory (`cloud/`).
The file is at the repository root.

- `cloud/app/config.py` — resolve `.env` from the repo root, absolutely
