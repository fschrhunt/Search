#!/bin/sh
# One repository entry point. Keep CI, contributor docs, and local checks on the
# same commands so no environment has a private definition of "green".
set -eu
cd "$(dirname "$0")"

usage() {
    echo "usage: ./x [build|fmt|lint|test|check|guard|shell|serve|stdio] [args...]" >&2
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
        [ "$#" -eq 0 ] || usage
        ./x fmt --check
        ./x lint
        ./x test
        ./x shell
        ./x guard
        ;;
    guard)
        sh scripts/guard.sh
        ;;
    # The shell scripts the release runs: a syntax error here fails a release,
    # not a pull request, so it belongs in check.
    shell)
        for script in x install.sh scripts/*.sh; do
            sh -n "$script"
        done
        echo "shell: ok"
        ;;
    serve)
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
