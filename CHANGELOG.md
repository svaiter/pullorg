# Changelog

## Unreleased

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
