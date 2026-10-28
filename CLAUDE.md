# pullorg

Rust pull parser for Org-mode modelled on pulldown-cmark, plus an HTML
renderer and a CLI. The reference implementation is Emacs' `org-element.el`;
the Org syntax document (https://orgmode.org/worg/org-syntax.html) is
secondary. When they disagree, Emacs wins.

## Layout

- `src/event.rs` public `Event`/`Tag`/`TagEnd` vocabulary and data structs.
- `src/element/` block-level parser: builds the element tree (`parse_document`).
- `src/object/` inline parser: `parse_objects(text, range, restriction, settings, out)`.
- `src/parser.rs` the `Parser` iterator: flattens the element tree, parses objects lazily.
- `src/html.rs` renderer (`push_html`, `write_html_fmt`, `write_html_io`).
- `src/bin/main.rs` CLI.
- `src/entities.rs` generated: `emacs --batch -Q -l org -l tools/gen-entities.el > src/entities.rs`.
- `tests/conformance.rs` + `tests/fixtures/*.org|json` comparison with Emacs.
- `tests/robustness.rs` random-input no-panic tests.

## Conventions

- Element ranges exclude trailing blank lines; object ranges exclude trailing
  whitespace; `Text` never contains `\n` (a `SoftBreak` is emitted) except
  inside raw blocks (src/example/export/comment, table.el).
- Affiliated keywords are `Event::Keyword { affiliated: true }` emitted right
  before the element's `Start`.
- Borrow from the input wherever possible; allocate only for transformed text.
- No `unsafe`, no new runtime dependencies without a reason.

## Workflow

- Oracle for any syntax question: `tools/org-parse.sh 'snippet'` prints the
  org-element tree Emacs produces. Decide behaviour from it, not from memory.
- Fixtures: add `tests/fixtures/NAME.org`, run `sh tools/regen-fixtures.sh`
  (needs Emacs), commit both files. `PULLORG_FIXTURE=name cargo test
  --test conformance` runs one. Never hand-edit the `.json`.
- Before committing: `cargo fmt`, `cargo test`, `cargo clippy --all-targets`.
- `ref/` (gitignored) may hold a local copy of the GPL Org Elisp sources for
  reading; never copy code or docstrings from it into the repo.
