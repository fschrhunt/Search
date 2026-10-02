# Architecture

search is a Cargo workspace with two crates. `crates/core` is the engine: it
names no terminal and no listener. `crates/cli` is the `search` binary: it puts
the engine behind a command line and a listener. `cli` depends on `core`, never
the reverse.

```
crates/core/  the engine (`search_core`)
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
  search/       mod.rs (Service, the facade), tools.rs (the MCP handlers)
crates/cli/   the binary
  args.rs       the command line
  run.rs        dispatch, and build the service with overrides
  stdio.rs      the stdio MCP transport
  http.rs       the JSON API, the router, and the auth middleware
  mcp.rs        the streamable HTTP MCP transport
```

## The rules

- **One facade.** `search::Service` is what both the MCP tools and the JSON API
  call. A change to search, fetch, or the index lands in the facade once and
  reaches both frontends.
- **The security-critical file is `fetch/guard.rs`.** Every class of address that
  can reach infrastructure must be classified private there, and
  `check_host` strips IPv6 brackets before parsing. Its tests carry the
  counterexamples; do not loosen them. `scripts/guard.sh` fails the build if the
  guard or its call sites move.
- **Shipped code denies panic sites.** `crates/core/src/lib.rs` denies
  `clippy::unwrap_used`, `expect_used`, `panic`, `unreachable`, and
  `indexing_slicing`. Every allowed site carries a `proof:` comment or a scoped
  `#[allow]` explaining why runtime input cannot reach it.
- **Providers are keyless by default**, and a provider's failure is reported in
  its `ProviderState` rather than failing the query.
- **Don't hardcode what a user might change.** A new user-facing behaviour is a
  config field, not a constant.
