# The key in .env reaches the running app

- apps/desktop/src-tauri/src/config.rs — `api_key()` falls back to a `.env`
  walk, the same ancestors build.rs searches.
- .env.example — say which of the two files is read when.
