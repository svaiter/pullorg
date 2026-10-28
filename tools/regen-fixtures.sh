#!/bin/sh
# Regenerate tests/fixtures/NAME.json from tests/fixtures/NAME.org using
# Emacs' org-element parser (the reference implementation).
#
# Usage: tools/regen-fixtures.sh            regenerate every fixture
#        tools/regen-fixtures.sh NAME ...   regenerate only these fixtures
#
# Requires `emacs` with Org on PATH (override with EMACS=/path/to/emacs).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
FIXTURES="$ROOT/tests/fixtures"
DUMP="$ROOT/tests/emacs/dump.el"
EMACS=${EMACS:-emacs}

command -v "$EMACS" >/dev/null 2>&1 || { echo "error: '$EMACS' not found" >&2; exit 1; }

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
if ! "$EMACS" --batch -Q -l org -l "$DUMP" -f pullorg-dump-batch "$@" >"$LOG" 2>&1; then
  echo "error: emacs failed:" >&2
  cat "$LOG" >&2
  exit 1
fi
version=$("$EMACS" --batch -Q -l org --eval '(princ (org-version))' 2>/dev/null | tail -n 1)
echo "regenerated $count fixture(s) with Org $version"
