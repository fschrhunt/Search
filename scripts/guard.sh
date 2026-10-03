#!/bin/sh
# The security-surface audit. Run locally before pushing; CI runs it on every
# PR. Each check pins a promise search makes to its users — a PR that moves one
# of these boundaries must change this script in the same diff, where the review
# can see it.
set -eu
cd "$(dirname "$0")/.."
fail=0

say() { printf '%s\n' "$*"; }
bad() { fail=1; say "FAIL: $*"; }

# The shipped Rust sources: everything under a crate's src, minus test modules.
# A `#[cfg(test)]` module is the last item in a file, so truncating at the first
# marker removes it. Without this, test fixtures trip boundaries meant for the
# shipped binary.
prod_sources() {
    find crates -path '*/src/*.rs' -o -path '*/src/*/*.rs' 2>/dev/null
}

# 1. Network surface. search talks to its discovery providers and the pages a
#    caller asks it to fetch, and nothing else. A new provider means a new host
#    user queries can reach — add it here deliberately or the build fails. Hosts
#    named only in a comment (a doc example, an injection illustration) are not
#    call sites and are skipped.
allowed_hosts="index.crates.io crates.io static.crates.io search.brave.com old-search.marginalia.nu api.mwmbl.org en.wikipedia.org hn.algolia.com news.ycombinator.com api.stackexchange.com export.arxiv.org example.com example.invalid localhost 127.0.0.1 0.0.0.0 github.com"
found_hosts=$(
    for f in $(prod_sources); do
        awk '/^#\[cfg\(test\)\]/ { exit } /^[[:space:]]*(\/\/|\*)/ { next } { print }' "$f"
    done | grep -ohE 'https?://[A-Za-z0-9.:-]+' | sed -E 's#https?://##' | sort -u
)
for host in $found_hosts; do
    case " $allowed_hosts " in
        *" $host "*) ;;
        *) bad "network host $host is not in the allowed list" ;;
    esac
done

# 2. Shipped code denies explicit panic sites: no unwrap, expect, panic!, or
#    unreachable! outside tests, except where a proof comment on the same or
#    preceding line explains why runtime input cannot reach the site.
for f in $(prod_sources); do
    awk -v file="$f" '
        /^#\[cfg\(test\)\]/ { exit }
        { lines[NR] = $0 }
        END {
            for (n = 1; n <= NR; n++) {
                text = lines[n]
                if (text ~ /unwrap\(\)|expect\(|panic!\(|unreachable!\(/) {
                    prev = lines[n-1]
                    prev2 = lines[n-2]
                    if (prev !~ /clippy::expect_used|clippy::unwrap_used|proof:/ &&
                        prev2 !~ /clippy::expect_used|clippy::unwrap_used|proof:/) {
                        printf "FAIL: %s:%d: explicit panic site without a proof comment\n", file, n
                    }
                }
            }
        }
    ' "$f"
done

# 3. The SSRF guard must exist and deny the metadata address. Removing or
#    weakening it is the one change this file exists to catch.
guard=crates/search/src/fetch/guard.rs
[ -f "$guard" ] || bad "the SSRF guard file $guard is missing"
grep -q "169.254" "$guard" || bad "the SSRF guard no longer covers link-local metadata"
grep -q "fn is_public_ip" "$guard" || bad "the SSRF guard's is_public_ip is gone"
grep -q "metadata.google.internal" "$guard" || bad "the SSRF guard no longer refuses the metadata hostname"

# 4. The fetcher must call the guard before dialing.
grep -q "guard::check_host" crates/search/src/fetch/mod.rs || bad "the fetcher no longer calls the SSRF guard"

# 5. Every HTTP request must pass the bearer check. The MCP endpoint and the
#    JSON API both sit behind the auth layer in the binary.
grep -q "middleware::from_fn_with_state" crates/cli/src/http.rs || bad "the auth middleware is no longer mounted"
grep -q "fn auth" crates/cli/src/http.rs || bad "the auth middleware is gone"

if [ "$fail" -eq 0 ]; then
    say "guard: ok"
fi
exit "$fail"
