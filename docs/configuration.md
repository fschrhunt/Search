# Configuration

A JSON file at `~/.config/search/search.json`, or the path in `$SEARCH_CONFIG`.
Fields are snake_case. Every field has a safe default, so a file names only what
it changes; a JSON object with unknown fields is rejected, so a typo fails at
startup rather than being ignored.

## Environment overrides

- `SEARCH_CONFIG` — the config file path.
- `SEARCH_ADDR` — the listen address, overriding `addr`.
- `SEARCH_DATA_DIR` — the data directory, overriding `data_dir`.
- The variable named by `token_env` (default `SEARCH_TOKEN`) — the bearer token.

The token is read from the environment first, then from a literal `token` in the
file. Prefer the environment so the file can be shared.

## Fields

```json
{
  "addr": "127.0.0.1:8642",
  "data_dir": "~/.local/share/search",
  "log": "info",
  "token_env": "SEARCH_TOKEN",
  "search": {
    "max_results": 10,
    "maxProviderTimeMs": 2000,
    "overallTimeoutMs": 8000,
    "cacheTtlMs": 300000
  },
  "fetch": {
    "timeoutMs": 15000,
    "max_bytes": 4194304,
    "max_redirects": 5,
    "cacheTtlMs": 600000,
    "allow_private": false,
    "max_concurrency": 8
  },
  "engines": {
    "enabled": ["brave", "wikipedia", "stackexchange"],
    "key_envs": { "brave": "BRAVE_SEARCH_KEY" }
  }
}
```

- **`addr`** — the listen address. A non-loopback address requires a token.
- **`data_dir`** — where the SQLite index lives.
- **`log`** — `debug`, `info`, `warn`, or `error`.
- **`search`** — how long any one provider may take, the ceiling for the whole
  fan-out, and how long a query answer is reused.
- **`fetch`** — the request deadline, the response size cap, the redirect limit,
  and how many fetches run at once. `index_fetched` defaults to true; set it
  false to stop adding fetched pages to the corpus. `allow_private` disables the
  SSRF guard — **tests and air-gapped mirrors only**.
- **`engines`** — restrict to a subset of providers with `enabled` (empty means
  every keyless provider), and name the environment variable holding a key for
  any provider that needs one with `key_envs`.

## Providers

Every provider runs without a key by default:

| Name | What it is |
| --- | --- |
| `brave` | A general web index, the primary English result source |
| `marginalia` | An independent index that favors non-commercial pages |
| `mwmbl` | A community-crawled index |
| `wikipedia` | Entities and concepts |
| `hackernews` | Developer and startup discussion |
| `stackexchange` | Concrete programming questions |
| `arxiv` | Research preprints |
