# Architecture

Search is a Cargo workspace with three packages. `crates/search` is the engine
library and has no terminal, listener, or protocol dependency. `crates/cli` is
the `search` executable and HTTP API. `crates/mcp` is the optional MCP adapter.
Both adapters depend on `search`; the engine depends on neither.

```
crates/search/  the engine (`search`)
  config/       the settings surface: settings.rs (the shape), defaults.rs
                (built-in values), load.rs (read, merge, validate)
  discovery/    the provider fan-out: mod.rs (Finding, Query, Response, the
                Provider trait), registry.rs (parallel fan-out, per-provider
                deadlines, reciprocal-rank fusion), parse.rs (HTML scanners),
                web.rs (the seven providers)
  fetch/        mod.rs (the guarded request, body caps, indexing), guard.rs (the
                SSRF guard and the DNS resolver), extract.rs (HTML to text),
                cache.rs (recent answers)
  index/        mod.rs (Store over SQLite), schema.rs (tables, triggers, the FTS
                query builder)
  service.rs    the public `Search` engine and its operations
crates/mcp/   the MCP tools and transports (`search_mcp`)
  lib.rs        tool definitions and stdio transport
  http.rs       streamable HTTP transport
crates/cli/   the command and HTTP API package (`cli`; binary `search`)
  args.rs       the command line
  run.rs        dispatch, and build the service with overrides
  http.rs       the JSON API, the router, and the auth middleware
```

## The rules

- **The engine stays independent.** `search::Search` is the in-process API.
  The CLI and MCP adapter depend on it; the engine does not depend on either.
- **The security-critical file is `fetch/guard.rs`.** Every class of address that
  can reach infrastructure must be classified private there, and
  `check_host` strips IPv6 brackets before parsing. Its tests carry the
  counterexamples; do not loosen them. `scripts/guard.sh` fails the build if the
  guard or its call sites move.
- **Shipped code denies panic sites.** `crates/search/src/lib.rs` denies
  `clippy::unwrap_used`, `expect_used`, `panic`, `unreachable`, and
  `indexing_slicing`. Every allowed site carries a `proof:` comment or a scoped
  `#[allow]` explaining why runtime input cannot reach it.
- **Providers are keyless by default**, and a provider's failure is reported in
  its `ProviderState` rather than failing the query.
- **Don't hardcode what a user might change.** A new user-facing behaviour is a
  config field, not a constant.
