# Emacs conformance tests

`tests/conformance.rs` checks that pulldown-org parses Org text the way
`org-element.el` (Emacs' reference implementation) does.

## How it works

- `tests/fixtures/NAME.org` is a small Org document, usually exercising one
  syntax feature.
- `tests/fixtures/NAME.json` is the org-element tree Emacs produced for it,
  written by `tests/emacs/dump.el`. The JSON files are checked in, so running
  the tests does not need Emacs.
- The test parses each `.org` file with `Parser::new(..).into_offset_iter()`,
  turns the events into the same JSON shape, normalizes both trees and
  compares them. All failing fixtures are reported in one run, each with a
  diff (left: Emacs, right: pulldown-org).

```sh
cargo test --test conformance                                     # all fixtures
PULLDOWN_ORG_FIXTURE=list-nested cargo test --test conformance    # one (or a,b,c)
```

A missing `.json` is a hard failure.

## Adding or regenerating fixtures

```sh
$EDITOR tests/fixtures/my-feature.org
sh tools/regen-fixtures.sh              # all fixtures
sh tools/regen-fixtures.sh my-feature   # just one
```

This runs
`emacs --batch -Q -l org -l tests/emacs/dump.el -f pulldown-org-dump-batch IN.org OUT.json ...`
(set `EMACS=` to pick a binary). The fixtures were generated with Org 9.8.3.
Look at the generated JSON before committing it: it is the expected output.
`tools/org-parse.sh -f FILE` prints the raw Emacs tree when in doubt.

## JSON shape

```json
{"type": "org-data", "children": [...]}
{"type": "paragraph", "begin": 0, "end": 12, "props": {...}, "children": [...]}
{"type": "plain-text", "value": "..."}
```

- `type` is the org-element type (`headline`, `plain-list`, `bold`, ...).
- `begin`/`end` are 0-based byte offsets. Elements: `:begin` to the start of
  the line after the last non-blank line (trailing blank lines are excluded).
  Objects: `:begin` to `:end - :post-blank`. Affiliated keywords are part of
  their element (`:begin` is on the first keyword).
- An object's post-blank whitespace (`*a* b`: the space) is added back as plain
  text, as in the event stream where it belongs to the next `Text`.
- `props` holds the type-specific properties (list in `dump.el`), with Emacs'
  property names. Symbols are strings, booleans are `true`/`false`, absent
  values are `null`, a headline priority is a 1-char string, an item counter
  is a number (`[@c]` is 3).
- Headline/inlinetask `title` and item `tag` are node arrays.
  `Start(Title)`/`Start(ItemTag)` events fill them.
- Planning and clock timestamps are timestamp nodes without positions.
- Citation and citation-reference `prefix`/`suffix` are raw text.
- `props.affiliated` lists the affiliated keywords as
  `{"key", "value", "optional"}`, sorted. The key is upper-cased and
  translated (`TBLNAME`/`SRCNAME`/... become `NAME`, `HEADERS` becomes
  `HEADER`, `RESULT` becomes `RESULTS`). The value is the trimmed raw text,
  so `CAPTION` is compared as raw text, not parsed. `optional` is the
  bracketed part of the dual keywords `CAPTION` and `RESULTS`. Emacs' side is
  read back from the lines between `:begin` and `:post-affiliated`.
- On the pulldown-org side, consecutive `Text`/`SoftBreak` events become one
  plain-text (a soft break is `\n`). For raw containers (src, example, export
  and comment blocks, `table.el` tables), the `Text` becomes the `value`
  property. The export block backend is upper-cased, as Emacs does. A radio
  target's `value` is taken from the source text in its range.

## Normalization (applied to both trees)

1. Adjacent plain-text nodes are merged.
2. The last plain-text child of any node list (children, title, tag) has its
   trailing whitespace trimmed, and is dropped if it ends up empty. Emacs
   keeps the final newline of a paragraph; pulldown-org may not.
3. `null` properties are dropped, so absent and null compare equal.
4. Trailing newlines of `value` are dropped for src, example, export and
   comment blocks, LaTeX environments and tables. Emacs keeps the final
   newline; pulldown-org does not.
5. Citation prefixes and suffixes are trimmed; empty ones are dropped.

Everything else is compared exactly, positions included.

## Self-tests

The converter has tests that do not need the parser:

- `converter::*` runs it on hand-built event streams.
- `roundtrip::*` turns every Emacs JSON into the event stream a perfect
  parser would emit, converts that back and checks it equals the JSON. This
  keeps `dump.el` and the converter in sync on every type and property the
  fixtures use.
- `fixture_json_is_well_formed` checks the oracle files themselves.
