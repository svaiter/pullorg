#!/bin/sh
# Compare the benchmarks of the working tree against a git ref.
#
#   sh tools/bench-compare.sh REF [cargo bench arguments...]
#   sh tools/bench-compare.sh main --bench parse -- parse/notes
#
# REF is built in a temporary git worktree and its benchmarks are saved as
# a Criterion baseline named after the ref; the working tree is then
# benchmarked against that baseline. The benchmark sources, Cargo.toml and
# Cargo.lock of the working tree are used on both sides, so only src/
# differs. REF is compiled in its own target directory (target/baseline),
# never in the working tree's: Cargo names artifacts by the path relative
# to the workspace root, so two checkouts sharing a target directory
# silently reuse each other's stale builds. Criterion's data does go to the
# shared target/criterion (CRITERION_HOME), so the baseline can be reused
# later with `cargo bench -- --baseline NAME`. The working tree itself is
# not touched.
#
# Arguments before `--` go to cargo (e.g. `--bench parse`); arguments after
# it go to the benchmark binary (e.g. a filter, `--quick`).
set -eu

if [ $# -lt 1 ]; then
    echo "usage: $0 REF [cargo bench arguments...]" >&2
    exit 2
fi
ref=$1
shift

root=$(git rev-parse --show-toplevel)
commit=$(git rev-parse --verify "$ref^{commit}")
name=$(printf '%s' "$ref" | tr -c 'A-Za-z0-9._-' '_')
target=${CARGO_TARGET_DIR:-$root/target}
worktree=$(mktemp -d "${TMPDIR:-/tmp}/pullorg-bench-XXXXXX")

cleanup() {
    git -C "$root" worktree remove --force "$worktree" 2>/dev/null || rm -rf "$worktree"
}
trap cleanup EXIT INT TERM

# Split the arguments at `--` into cargo arguments and bench arguments.
cargo_args=""
bench_args=""
seen_sep=0
for arg in "$@"; do
    if [ "$seen_sep" = 0 ] && [ "$arg" = "--" ]; then
        seen_sep=1
        continue
    fi
    if [ "$seen_sep" = 0 ]; then
        cargo_args="$cargo_args $arg"
    else
        bench_args="$bench_args $arg"
    fi
done

echo "==> baseline '$name' from $ref ($commit)"
git -C "$root" worktree add --detach --quiet "$worktree" "$commit"
# The suite, manifest and lockfile are taken from the working tree so both
# sides run the same benchmarks with the same dependencies, even when REF
# predates a benchmark or the suite itself, and so are tests/fixtures (the
# `fixtures` corpus); REF supplies src/ only.
rm -rf "$worktree/benches"
cp -R "$root/benches" "$worktree/benches"
cp "$root/Cargo.toml" "$root/Cargo.lock" "$worktree/"
rm -rf "$worktree/tests/fixtures"
cp -R "$root/tests/fixtures" "$worktree/tests/fixtures"
# shellcheck disable=SC2086
(cd "$worktree" && CARGO_TARGET_DIR="$target/baseline" CRITERION_HOME="$target/criterion" \
    cargo bench $cargo_args -- --save-baseline "$name" $bench_args)

echo "==> working tree against '$name'"
# shellcheck disable=SC2086
(cd "$root" && CARGO_TARGET_DIR="$target" CRITERION_HOME="$target/criterion" \
    cargo bench $cargo_args -- --baseline "$name" $bench_args)
