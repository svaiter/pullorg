#!/bin/sh
# Regenerate tests/fixtures/NAME.json from tests/fixtures/NAME.org using
# Emacs' org-element parser (the reference implementation).
#
# Usage: tools/regen-fixtures.sh            regenerate every fixture
#        tools/regen-fixtures.sh NAME ...   regenerate only these fixtures
#
# Requires `emacs` on PATH (override with EMACS=/path/to/emacs). The Org
# version must match tests/emacs/ORG-VERSION, the version the committed
# fixtures were generated with; point ORG_LOAD_PATH at the `lisp/` directory
# of an Org checkout (after `make autoloads`) to use a version other than the
# one bundled with Emacs, or set ALLOW_ORG_MISMATCH=1 to regenerate anyway
# (then update ORG-VERSION and review every changed fixture).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
FIXTURES="$ROOT/tests/fixtures"
DUMP="$ROOT/tests/emacs/dump.el"
EMACS=${EMACS:-emacs}
WANT=$(tr -d ' \n' < "$ROOT/tests/emacs/ORG-VERSION")

command -v "$EMACS" >/dev/null 2>&1 || { echo "error: '$EMACS' not found" >&2; exit 1; }

# Make Emacs load the pinned Org before anything else (ORG_LOAD_PATH must not
# contain spaces).
ORG_ARGS=""
[ -n "${ORG_LOAD_PATH:-}" ] && ORG_ARGS="-L $ORG_LOAD_PATH"
# shellcheck disable=SC2086
emacs_org() { "$EMACS" --batch -Q $ORG_ARGS "$@"; }
version=$(emacs_org -l org --eval '(princ (org-version))' 2>/dev/null | tail -n 1)
if [ "$version" != "$WANT" ] && [ -z "${ALLOW_ORG_MISMATCH:-}" ]; then
  echo "error: Org $version found, fixtures are generated with Org $WANT" >&2
  echo "       (set ORG_LOAD_PATH to an Org $WANT checkout, or ALLOW_ORG_MISMATCH=1)" >&2
  exit 1
fi

if [ $# -eq 0 ]; then
  set -- "$FIXTURES"/*.org
  [ -e "$1" ] || { echo "error: no fixtures in $FIXTURES" >&2; exit 1; }
else
  names="$*"
  set --
  for n in $names; do
    n=${n%.org}; n=${n%.json}; n=$(basename "$n")
    [ -e "$FIXTURES/$n.org" ] || { echo "error: no fixture $FIXTURES/$n.org" >&2; exit 1; }
    set -- "$@" "$FIXTURES/$n.org"
  done
fi

count=$#
for org in "$@"; do
  shift
  set -- "$@" "$org" "${org%.org}.json"
done

LOG=$(mktemp -t regen-fixtures.XXXXXX)
trap 'rm -f "$LOG"' EXIT
if ! emacs_org -l org -l "$DUMP" -f pullorg-dump-batch "$@" >"$LOG" 2>&1; then
  echo "error: emacs failed:" >&2
  cat "$LOG" >&2
  exit 1
fi
echo "regenerated $count fixture(s) with Org $version"
