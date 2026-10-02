# Changelog

## Unreleased

- Extraction keeps the article and drops the cruft: ads, cookie banners,
  related rails, and hidden text are removed, and links and images are
  neutralized so page content cannot exfiltrate. A page that only redirects
  elsewhere is now followed to its target.
- `web_fetch` takes a `query` and returns the passages that match it instead of
  the whole page, and `max_characters` to bound the answer. The unused
  `objective` parameter is gone.
- Fetched pages carry byline, published time, and site, for citation.
- A finding's `providers` is a JSON array, not a joined string.

## Unreleased (before this change)

- The first Rust implementation: one binary serving multi-provider discovery, a
  hardened fetcher, a private full-text index, and MCP over stdio and HTTP.
- Release tooling: `scripts/release.sh` (name the changelog section, then tag),
  `scripts/formula.sh` (the Homebrew formula), and `install.sh` (the
  checksum-verified installer), with CI and a tag workflow that builds archives,
  attests provenance, updates the formula, and installs the release.
- Documentation and contribution: `CONTRIBUTING.md`, `docs/` for users and
  `docs/contributing/` for the project, issue and pull-request templates, and a
  `./x check` that includes a shell-syntax check.
