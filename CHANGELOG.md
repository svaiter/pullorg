# Changelog

## Unreleased

- Index closing delimiters lazily and share their positions across element
  searches, avoiding repeated suffix scans for unterminated blocks, drawers
  and LaTeX environments, including openers inside lists.
- Build and traverse block structure with an explicit stack and flat element
  arena, avoiding stack overflows for deeply nested blocks without imposing a
  block depth limit. Subscript and superscript validation now stops at Org's
  existing supported depth before recursing further.

## 0.1.0 — 2026-10-29

Initial release.

- Pull parser covering the whole Org syntax: headlines, sections, planning,
  property drawers, drawers, blocks, dynamic blocks, footnotes, plain lists,
  tables, LaTeX environments, fixed-width, comments, keywords and affiliated
  keywords, rules, diary sexps, babel calls, clocks, paragraphs, inline tasks;
  emphasis, code/verbatim, entities, LaTeX fragments, links, timestamps,
  footnote references, sub/superscripts, targets and radio targets, macros,
  export snippets, inline src blocks and babel calls, statistics cookies,
  line breaks, citations.
- In-buffer settings: `#+TODO`/`#+SEQ_TODO`/`#+TYP_TODO`, `#+OPTIONS: ^:`,
  `#+LINK`, `#+STARTUP: odd`.
- HTML renderer and `pullorg` command-line tool.
- Conformance suite comparing the event stream with Emacs'
  `org-element-parse-buffer` (Org 9.8.3) on 47 fixtures; also verified on
  the Org manual, the Org guide and ORG-NEWS.
- `Event`, `Tag`, `TagEnd` and `Options` are `#[non_exhaustive]`, so new
  syntax and options can be added without a breaking release: match them
  with a `_` arm, and build `Options` from `Options::default()`.
- Benchmark suite under `benches/`: Criterion targets for parsing, HTML
  rendering, each element and object kind, and scaling on adversarial
  inputs, plus a deterministic allocation report (`cargo bench --bench
  alloc`). `tools/bench-compare.sh` compares the working tree with a git
  ref.
- `Options::link_types` registers link types, as `org-link-set-parameters`
  does in Emacs: `[[media:key]]`, `<media:key>` and a plain `media:key` are
  then links of type `media` instead of fuzzy links or text.
- `Parser::new_inline` / `Parser::new_inline_ext` parse a string as objects
  only (a secondary string), e.g. the parameters of a special block.
- The HTML renderer removes the common indentation of source and example
  blocks (`org-remove-indentation`), unless the `-i` switch is given.
- Documented how the event stream maps to `org-element`'s `:contents-begin`
  of items, `:post-blank`, table cell ranges and raw block parameters.
