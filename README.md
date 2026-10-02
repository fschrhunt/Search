# Search

Self-hosted, agent-first web search in one binary. It fans a query out to
several independent search providers in parallel, merges and reranks the
results, fetches pages through a hardened reader, and indexes everything it
reads into a private corpus you can search offline.

Built to run on a personal server and be reached over a private network such as
a tailnet. There are no accounts, no usage quotas, and no telemetry.

## What it is

- **One static binary.** Go, no cgo, SQLite compiled in. Nothing else to install.
- **Keyless by default.** Works out of the box against providers that need no
  API key. Providers that offer a key can be enabled with one.
- **Agent-shaped output.** Every response reports, per provider, whether it
  answered, timed out, or failed, so an empty result is never mistaken for a
  broken one.
- **A private corpus that grows from use.** Every page `fetch` reads is stored
  and full-text indexed. Repeated reading is instant and independent of any
  upstream provider.

## Status

Early. The core (discovery, fetch, index, HTTP API) works and the pieces are
tested. MCP and packaging are next. Until then, treat it as a moving target.

## Build

```sh
go build ./cmd/search
```

## Run

```sh
export SEARCH_TOKEN=$(openssl rand -hex 32)
./search
```

By default it binds `127.0.0.1:8642`. To expose it on a private interface (for
example a tailnet address), set `addr` and keep the token set — the service
refuses to bind a non-loopback address without one.

## API

All requests carry `Authorization: Bearer $SEARCH_TOKEN`.

```
GET  /healthz                 liveness
GET  /v1/status               providers, corpus size, version
GET  /v1/search?q=...         discover across providers
GET  /v1/index?q=...          search only what has been fetched already
POST /v1/fetch {"urls":[...]} read pages into text, and index them
```

## Configuration

A JSON file at `~/.config/search/search.json` (or `$SEARCH_CONFIG`), and the
environment variables `SEARCH_ADDR`, `SEARCH_DATA_DIR`, and the token variable
named by `tokenEnv` (default `SEARCH_TOKEN`). Every field has a safe default; a
config file only overrides what it names.

## Security

The fetcher treats every URL as hostile. It refuses private, loopback,
link-local and metadata addresses, re-checks each redirect hop, dials only the
address it vetted, caps response size, and enforces a deadline. The service
authenticates every request and refuses to bind a non-loopback address without a
token. It is still meant to sit behind a private network: run it on loopback, or
on a tailnet interface, not on the open internet.

## License

MIT
