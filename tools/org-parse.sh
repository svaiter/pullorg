#!/bin/sh
# Print the org-element parse tree Emacs produces for an Org snippet.
#
# Usage: tools/org-parse.sh 'some *org* text'      (snippet as argument)
#        tools/org-parse.sh -f file.org            (from a file)
#        echo '...' | tools/org-parse.sh           (from stdin)
set -e
if [ "$1" = "-f" ]; then
  INPUT=$(cat "$2")
elif [ $# -gt 0 ]; then
  INPUT="$1"
else
  INPUT=$(cat)
fi
TMP=$(mktemp -t orgparse.XXXXXX)
printf '%s' "$INPUT" > "$TMP"
emacs --batch -Q -l org -l pp --eval "
(progn
  (setq org-inhibit-startup t)
  (with-temp-buffer
    (insert-file-contents \"$TMP\")
    (org-mode)
    (let ((print-length nil) (print-level nil) (pp-default-function 'pp-28))
      (pp (org-element-parse-buffer)))))" 2>/dev/null
rm -f "$TMP"
