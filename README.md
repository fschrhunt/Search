<div align="center">
  <img src="assets/white/lockup.svg#gh-dark-mode-only" alt="Search" height="52">
  <img src="assets/black/lockup.svg#gh-light-mode-only" alt="Search" height="52">

  <h3>Search the web. Build your own index as you go.</h3>

  <p>Self-hosted web search for people and AI agents.<br>
  Find pages across independent providers, read them cleanly, and keep what you fetch in a private index.</p>

  <p>
    <a href="docs/install.md">Install</a> ·
    <a href="docs/usage.md">Usage</a> ·
    <a href="docs/configuration.md">Configuration</a>
  </p>
</div>

<br>

## Find, read, keep

- **Find:** query several independent providers in parallel and merge their
  results. Keyless providers work out of the box; a failed provider is reported
  rather than hidden.
- **Read:** fetch pages through an SSRF-protected reader that strips page
  clutter. Ask for passages relevant to a query instead of a whole article.
- **Keep:** fetched pages join a local SQLite full-text index. Search blends
  local matches with live results, so useful pages remain searchable when
  providers are unavailable. Size and age limits keep the index bounded.

Index search and fetch indexing can each be switched off. Seeded hosts refresh
only when configured, and only through the explicit refresh command—Search does
not crawl the web on its own.

## Start

Build from source with Rust:

```sh
git clone https://github.com/fschrhunt/search
cd search
cargo build --release
```

Search from your terminal—no server required:

```sh
./target/release/search "rust async runtime"
./target/release/search fetch https://www.rust-lang.org -query "async"
./target/release/search index "async runtime" -json
```

Or start the HTTP API and MCP server:

```sh
export SEARCH_TOKEN="$(openssl rand -hex 32)"
./target/release/search serve
```

HTTP and MCP-over-HTTP require a bearer token, even on loopback. The server
listens on `127.0.0.1:8642` by default; Search also refuses a non-loopback bind
without a token. Stdio MCP and one-shot CLI commands do not need one. See
[installation](docs/install.md) for release and package-manager options.

## Use it with an agent

With no arguments, `search` serves MCP over stdio. Add it to your MCP client:

```json
{
  "mcp": {
    "servers": {
      "search": { "command": ["search"] }
    }
  }
}
```

The server exposes two tools: `web_search` for discovery and `web_fetch` for
clean, query-focused reading. For a shared server, use MCP over HTTP instead.

## One engine, more surfaces

| Surface | Use it for |
| --- | --- |
| CLI | One-shot search, fetch, and local-index queries |
| Rust | Use `search::Search` as an in-process engine |
| HTTP | Integrate with scripts and services; includes status, search, index, and fetch endpoints |
| MCP | Give an agent the `web_search` and `web_fetch` tools |

The engine API lives in the `search` workspace crate. The `cli` and `mcp`
workspace packages are separate adapters; MCP dependencies are not part of the
engine crate. In Rust, start with `use search::{Config, Search};` and open a
configured engine with `Search::open(config)`.

## Make it yours

Settings live in `~/.config/search/search.json` or the file named by
`SEARCH_CONFIG`. Defaults are useful; turn features off or tune them as needed.

```json
{
  "search": { "use_index": true },
  "fetch": { "index_fetched": true, "index_text_chars": 40000 },
  "index": {
    "max_size_mb": 512,
    "max_age_days": 180,
    "refresh_hosts": [],
    "refresh_after_days": 7
  }
}
```

Set `search.use_index` or `fetch.index_fetched` to `false` to disable that
behavior. Set `max_size_mb` or `max_age_days` to `0` for no limit. See the full
[configuration reference](docs/configuration.md).

## API

Every HTTP request uses `Authorization: Bearer $SEARCH_TOKEN`.

```text
GET  /healthz                 liveness
GET  /v1/status               providers, corpus size, version
GET  /v1/search?q=...         search providers and local index
GET  /v1/index?q=...          search only the local index
POST /v1/fetch {"urls":[...]} fetch pages and optionally index them
POST /mcp                     MCP over streamable HTTP
```

## Security

Fetched URLs are untrusted. Search blocks private and metadata-network
destinations, re-checks redirects, limits response size, and enforces deadlines.
Keep a network-facing instance on a private network, behind authentication; do
not expose it to the public internet. Read the [security notes](docs/security.md)
before deployment.

## Project

Search is early software. See the [docs](docs/README.md), [changelog](CHANGELOG.md),
and [contributing guide](CONTRIBUTING.md). Licensed under MIT.
