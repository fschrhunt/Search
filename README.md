# Search

Self-hosted, agent-first web search in one binary. It fans a query out to
several independent search providers in parallel, merges and reranks the
results, fetches pages through a hardened reader, and indexes everything it
reads into a private corpus you can search offline.

Built to run on a personal server and be reached over a private network such as
a tailnet. There are no accounts, no usage quotas, and no telemetry.

## What it is

- **One static binary.** Rust, no system dependencies, SQLite compiled in.
- **Keyless by default.** Works out of the box against providers that need no
  API key.
- **Agent-shaped output.** Every response reports, per provider, whether it
  answered, timed out, or failed, so an empty result is never mistaken for a
  broken one.
- **A private corpus that grows from use.** Every page `fetch` reads is stored
  and full-text indexed. Repeated reading is instant and independent of any
  upstream provider.
- **MCP over stdio and HTTP.** The same two tools — `web_search` and
  `web_fetch` — for a locally spawned agent or one that reaches it over the
  network.

## Status

Early. The engine, the fetcher, the index, and both MCP transports work and are
tested. Packaging and release tooling are next.

## Build

```sh
cargo build --release
```

## Run

```sh
export SEARCH_TOKEN=$(openssl rand -hex 32)
./target/release/search serve
```

By default it binds `127.0.0.1:8642`. To expose it on a private interface, set
`addr` in the config and keep the token set — the service refuses to bind a
non-loopback address without one.

An agent can also spawn it directly; with no subcommand it serves MCP over
stdio:

```json
{ "mcp": { "servers": { "search": { "command": ["search"] } } } }
```

## API

All requests carry `Authorization: Bearer $SEARCH_TOKEN`.

```
GET  /healthz                 liveness
GET  /v1/status               providers, corpus size, version
GET  /v1/search?q=...         discover across providers
GET  /v1/index?q=...          search only what has been fetched already
POST /v1/fetch {"urls":[...]} read pages into text, and index them
POST /mcp                     MCP over streamable HTTP
```

## Configuration

A JSON file at `~/.config/search/search.json` (or `$SEARCH_CONFIG`), and the
environment variables `SEARCH_ADDR`, `SEARCH_DATA_DIR`, and the token variable
named by `token_env` (default `SEARCH_TOKEN`). Fields are snake_case; every
field has a safe default, so a config file names only what it changes.

```json
{
  "addr": "100.64.0.1:8642",
  "data_dir": "~/.local/share/search",
  "engines": { "enabled": ["brave", "wikipedia", "stackexchange"] }
}
```

## Security

The fetcher treats every URL as hostile. It refuses private, loopback,
link-local and metadata addresses — including the IPv6 forms that embed them —
re-checks each redirect hop, caps response size, and enforces a deadline. The
service authenticates every request, the MCP endpoint included, and refuses to
bind a non-loopback address without a token. It is still meant to sit behind a
private network: run it on loopback, or on a tailnet interface, not on the open
internet. See `SECURITY.md`.

## License

MIT
