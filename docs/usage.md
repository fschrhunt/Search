# Usage

search is a web search service. It fans a query out to several independent
providers, merges and reranks the results, reads pages through a hardened
fetcher, and indexes everything it reads into a private corpus.

It speaks the Model Context Protocol, so an agent uses it as two tools —
`web_search` and `web_fetch` — and it also offers a small JSON API.

## Set a token

The service authenticates every request. Set a token in its environment:

```sh
export SEARCH_TOKEN=$(openssl rand -hex 32)
```

A non-loopback bind refuses to start without one.

## Serve MCP over stdio

With no subcommand, search serves MCP over stdin/stdout. This is what an agent
spawns:

```json
{ "mcp": { "servers": { "search": { "command": ["search"] } } } }
```

## Serve over HTTP

```sh
search serve
```

This binds `127.0.0.1:8642` and serves the JSON API and the MCP endpoint
(`/mcp`) on one listener. Set `addr` in the config to reach it on a private
interface such as a tailnet address.

## The JSON API

All requests carry `Authorization: Bearer $SEARCH_TOKEN`.

```
GET  /healthz                 liveness
GET  /v1/status               providers, corpus size, version
GET  /v1/search?q=...         discover across providers
GET  /v1/index?q=...          search only what has been fetched already
POST /v1/fetch {"urls":[...]} read pages into text, and index them
POST /mcp                     MCP over streamable HTTP
```

`GET /v1/search` also takes `limit` (1–50) and `providers` (a comma-separated
list of provider names). Every answer reports, per provider, whether it answered,
timed out, or failed.

## The MCP tools

- **`web_search`** takes one to five queries, an optional objective, a result
  limit, and an optional provider list. It returns ranked results with title,
  URL, and snippet.
- **`web_fetch`** takes one to ten URLs and an optional objective. It returns
  each page as clean text and adds it to the private index. Private and
  link-local addresses are refused.

## The private corpus

Every page `web_fetch` reads is stored and full-text indexed. `GET /v1/index`
searches that corpus offline, so a page read once is instant to read again and
independent of any provider. The corpus lives in `data_dir` as a SQLite
database.
