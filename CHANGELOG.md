# Changelog

## Unreleased

- `Options::link_types` registers link types, as `org-link-set-parameters`
  does in Emacs: `[[media:key]]`, `<media:key>` and a plain `media:key` are
  then links of type `media` instead of fuzzy links or text.
- `Parser::new_inline` / `Parser::new_inline_ext` parse a string as objects
  only (a secondary string), e.g. the parameters of a special block.
- The HTML renderer removes the common indentation of source and example
  blocks (`org-remove-indentation`), unless the `-i` switch is given.
- Documented how the event stream maps to `org-element`'s `:contents-begin`
  of items, `:post-blank`, table cell ranges and raw block parameters.

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
