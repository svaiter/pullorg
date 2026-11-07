#!/bin/sh
# Compare pullorg's parse time with Emacs' org-element-parse-buffer on the
# same inputs.
#
#   sh tools/bench-vs-emacs.sh [-n SAMPLES] [-w WARMUP] [-o DIR] [FILTER]
#
# 1. `cargo bench --bench vs_emacs` regenerates the benchmark corpora (every
#    document corpus of benches/common at 4, 16, 64 and 256 KiB, plus the
#    files of PULLORG_BENCH_CORPUS when set) into DIR/corpus and times
#    pullorg on them (DIR/pullorg.tsv).
# 2. tools/bench-emacs.el times Emacs on the same files (DIR/emacs.tsv).
# 3. The two are joined into DIR/report.txt, also printed.
#
# Nothing here is checked in: the corpora are rebuilt on every run into DIR
# (default target/vs-emacs), replacing the previous run's files. FILTER keeps the corpora
# whose name contains it (e.g. `64k`, `org-manual`). Defaults: 20 samples,
# 3 warm-up samples, on both sides.
#
# Requires `emacs` on PATH (override with EMACS=/path/to/emacs). As for the
# fixtures, the Org version must match tests/emacs/ORG-VERSION: point
# ORG_LOAD_PATH at the `lisp/` directory of an Org checkout to use another
# Org (a natively compiled one, for instance), or set ALLOW_ORG_MISMATCH=1.
# The report says whether Org ran byte-compiled or natively compiled.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
EMACS=${EMACS:-emacs}
WANT=$(tr -d ' \n' < "$ROOT/tests/emacs/ORG-VERSION")
SAMPLES=20
WARMUP=3
OUT="$ROOT/target/vs-emacs"
while getopts n:w:o: opt; do
  case $opt in
    n) SAMPLES=$OPTARG ;;
    w) WARMUP=$OPTARG ;;
    o) OUT=$OPTARG ;;
    *) echo "usage: $0 [-n SAMPLES] [-w WARMUP] [-o DIR] [FILTER]" >&2; exit 2 ;;
  esac
done
shift $((OPTIND - 1))
FILTER=${1:-}

command -v "$EMACS" >/dev/null 2>&1 || { echo "error: '$EMACS' not found" >&2; exit 1; }
ORG_ARGS=""
[ -n "${ORG_LOAD_PATH:-}" ] && ORG_ARGS="-L $ORG_LOAD_PATH"
# shellcheck disable=SC2086
emacs_org() { "$EMACS" --batch -Q $ORG_ARGS "$@"; }
version=$(emacs_org -l org --eval '(princ (org-version))' 2>/dev/null | tail -n 1)
if [ "$version" != "$WANT" ] && [ -z "${ALLOW_ORG_MISMATCH:-}" ]; then
  echo "error: Org $version found, the fixtures pin Org $WANT" >&2
  echo "       (set ORG_LOAD_PATH to an Org $WANT checkout, or ALLOW_ORG_MISMATCH=1)" >&2
  exit 1
fi

mkdir -p "$OUT"
rm -rf "$OUT/corpus" "$OUT/pullorg.tsv" "$OUT/emacs.tsv" "$OUT/report.txt"
OUT=$(cd "$OUT" && pwd)

echo "== pullorg" >&2
(cd "$ROOT" && cargo bench -q --bench vs_emacs -- \
  --out "$OUT" --samples "$SAMPLES" --warmup "$WARMUP" ${FILTER:+"$FILTER"})
[ -n "$(ls "$OUT/corpus" 2>/dev/null)" ] || { echo "error: no corpus matches '$FILTER'" >&2; exit 1; }

echo "== emacs" >&2
# Org files may name a #+SETUPFILE relative to themselves; run from there.
(cd "$OUT/corpus" && emacs_org -l org -l "$ROOT/tools/bench-emacs.el" \
  -f pullorg-bench-batch "$OUT/corpus" "$OUT/emacs.tsv" "$SAMPLES" "$WARMUP") 2>&1 |
  grep '^emacs ' >&2 || true
[ -s "$OUT/emacs.tsv" ] || { echo "error: emacs produced no report" >&2; exit 1; }

cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null ||
  sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo 2>/dev/null | head -n 1 || true)
{
  echo "pullorg $(cd "$ROOT" && git describe --always --dirty 2>/dev/null || echo '?')," \
    "$(rustc --version)"
  sed -n 's/^# //p' "$OUT/emacs.tsv"
  echo "$(uname -sm), ${cpu:-unknown CPU}, $(date -u '+%Y-%m-%d %H:%M UTC')"
  echo "$SAMPLES samples after $WARMUP warm-up, median time per parse; range is min-max"
  echo
  awk -F '\t' '
    FNR == NR { if ($0 !~ /^#/) emacs[$1 FS $3] = $0; next }
    {
      key = $1 FS $3
      if (!(key in emacs)) next
      split(emacs[key], e, FS)
      mib = $2 / 1048576
      printf "%-22s %8.1f  %-8s  %9.3f (%.3f-%.3f)  %9.3f (%.3f-%.3f) gc %6.3f  %6.1fx  %7.1f %6.1f\n",
        $1, $2 / 1024, $3, $4 * 1e3, $5 * 1e3, $6 * 1e3,
        e[4] * 1e3, e[5] * 1e3, e[6] * 1e3, e[7] * 1e3,
        e[4] / $4, mib / $4, mib / e[4]
    }' "$OUT/emacs.tsv" "$OUT/pullorg.tsv" |
    { printf "%-22s %8s  %-8s  %-27s  %-37s  %7s  %7s %6s\n" \
        corpus KiB level "pullorg ms" "emacs ms (gc ms)" speedup "MiB/s p" "MiB/s e"
      cat; }
} | tee "$OUT/report.txt"
