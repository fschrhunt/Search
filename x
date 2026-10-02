#!/bin/sh
# One repository entry point. Keep CI, contributor docs, and local checks on the
# same commands so no environment has a private definition of "green".
set -eu
cd "$(dirname "$0")"

usage() {
    echo "usage: ./x [build|fmt|lint|test|check|guard|serve|stdio] [args...]" >&2
    exit 2
}

command=${1:-check}
if [ "$#" -gt 0 ]; then
    shift
fi

case "$command" in
    build)
        cargo build --locked "$@"
        ;;
    fmt)
        if [ "${1:-}" = "--check" ]; then
            cargo fmt --all --check
        else
            cargo fmt --all
        fi
        ;;
    lint)
        cargo clippy --locked --all-targets --all-features "$@" -- -D warnings
        ;;
    test)
        cargo test --locked --workspace "$@"
        ;;
    check)
        ./x fmt --check
        ./x lint
        ./x test
        ./x guard
        ;;
    guard)
        sh scripts/guard.sh
        ;;
    serve)
        shift_ok=${1:-}
        cargo build --locked
        exec ./target/debug/search serve "$@"
        ;;
    stdio)
        cargo build --locked
        exec ./target/debug/search "$@"
        ;;
    *)
        usage
        ;;
esac
