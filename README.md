# pullorg

A pull parser for [Org-mode](https://orgmode.org) written in Rust, in the
style of [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark).
It ships as a library and as a small command-line tool that renders Org to
HTML.

The parser follows the [Org syntax](https://orgmode.org/worg/org-syntax.html)
document and, where the two differ, the reference implementation in Emacs
(`org-element.el`). Its output is tested against Emacs: the test suite compares
the event stream with the parse tree produced by `org-element-parse-buffer` on
a corpus of fixtures.

## Provenance

This project was bootstrapped entirely by AI. The initial code base, tests,
tooling and documentation were written by Claude (Claude Fable 5.1, through
Claude Code) from a short specification, with the Emacs implementation as the
behavioural reference. Human review of the code is ongoing; treat the first
releases accordingly and read the test suite as the most reliable statement of
what is verified.

## Library

```rust
use pullorg::{html, Parser};

let source = "* A headline\nSome /italic/ text with a [[https://orgmode.org][link]].";
let parser = Parser::new(source);

let mut out = String::new();
html::push_html(&mut out, parser);
```

The parser is an `Iterator<Item = Event>`. Containers (headlines, paragraphs,
lists, blocks, markup, links, ...) are delimited by `Event::Start(Tag)` and
`Event::End(TagEnd)`; everything else (text, timestamps, entities, keywords,
...) is a leaf event.

```rust
use pullorg::{Event, Parser, Tag};

for event in Parser::new("* TODO Write docs :work:") {
    match event {
        Event::Start(Tag::Headline(h)) => {
            println!("level {} todo={:?} tags={:?}", h.level, h.todo_keyword, h.tags);
        }
        Event::Text(text) => println!("text: {text}"),
        _ => {}
    }
}
```

Use `Parser::into_offset_iter()` to get the byte range of each event in the
source, and `Parser::new_ext(source, Options)` to change the TODO keywords,
the interpretation of `_`/`^`, to enable inline tasks or to register link
types. In-buffer settings (`#+TODO:`, `#+OPTIONS: ^:nil`, `#+LINK:`)
override the options, as in Emacs.

```rust
use pullorg::{Options, Parser};

// Like `org-link-set-parameters`: `[[media:key]]` is now a link of type
// `media` with path `key` (instead of a `fuzzy` link), and so is a plain
// `media:key` in text.
let mut options = Options::default();
options.link_types = vec!["media".to_string()];
let parser = Parser::new_ext("[[media:key][a picture]]", options);
```

`Parser::new_inline(text)` parses a string as objects only, the way Emacs
parses a secondary string: use it for the raw strings the parser hands over
as written, such as the parameters of a special block or a `#+CAPTION:`
value.

### How the stream is shaped

- A headline is `Start(Headline)`, then `Start(Title)` ... `End(Title)` holding
  the title objects, then an optional `Start(Section)` ... `End(Section)` with
  the content before the first sub-headline, then the sub-headlines, then
  `End(Headline)`. Headlines nest, like the Org outline.
- A list item is `Start(Item)`, an optional `Start(ItemTag)` ... `End(ItemTag)`
  for descriptive lists, then its elements, then `End(Item)`.
- Affiliated keywords (`#+NAME:`, `#+CAPTION:`, `#+ATTR_HTML:` ...) are
  `Event::Keyword` events with `affiliated: true`, emitted right before the
  `Start` of the element they apply to.
- Source, example, export and comment blocks contain exactly one `Text` event
  with the raw body, indentation included (`org-element`'s `:value`); the
  HTML renderer removes the common indentation, like `ox-html`.
- Inside paragraphs `Text` never spans a line; a `SoftBreak` is emitted for
  each newline, like pulldown-cmark. Join the `Text`/`SoftBreak` run to get
  the plain text `org-element` would hold in one string.
- The whitespace after an object (`org-element`'s `:post-blank`) begins the
  `Text` that follows the object.
- Element ranges (from `into_offset_iter`) exclude the trailing blank lines
  that `org-element`'s `:end` includes; object ranges exclude trailing
  whitespace. The range of `Start(Item)` begins at the bullet; the item's
  contents start at the first event after `Start(Item)` (or `End(ItemTag)`).
  A `TableCell` range ends after the cell's closing `|`.
- Special block parameters (`#+begin_theorem Title :open`) are one raw
  string, as in `org-element`; parse it with `Parser::new_inline` if it
  holds markup.

## Command line

```
pullorg README.org            # HTML fragment on stdout
pullorg README.org -o out.html --standalone --mathjax
pullorg --events notes.org    # dump the event stream
cat file.org | pullorg
```

Install with `cargo install pullorg`. The CLI is behind the default
`cli` feature; use `default-features = false` when depending on the library
only.

## Testing against Emacs

`tests/fixtures/*.org` are parsed by Emacs (`tests/emacs/dump.el`) into
`tests/fixtures/*.json` reference trees, checked in so the test suite runs
without Emacs. `sh tools/regen-fixtures.sh` regenerates them with the Org
version pinned in `tests/emacs/ORG-VERSION`; CI rebuilds that exact Org from
its git tag and fails if the committed trees drift. `tools/org-parse.sh 'snippet'`
prints the raw `org-element` tree for any snippet, which is the quickest way
to settle a question about the syntax.

## Benchmarks

`cargo bench` runs a Criterion suite over generated corpora (a realistic
notes file, plain prose, markup-dense paragraphs, element-heavy documents,
adversarial inputs at several sizes) and `cargo bench --bench alloc` prints
a deterministic allocation report. `benches/README.md` describes the
targets, the corpora, and how to compare two commits.

## Status

Early, but complete: every element and object of the Org syntax is parsed,
including timestamps, footnotes, citations, tables, drawers, inline tasks and
in-buffer TODO keywords. The event stream matches Emacs' `org-element` tree
on the 47 checked-in fixtures and on large real-world documents (the Org
manual, the Org guide and ORG-NEWS); the manual (900 KB) parses and renders
in about 20 ms. Expect API changes before 1.0.

Not yet handled: macro re-parsing (`#+MACRO` definitions and the built-in
macros expand to plain text in the HTML renderer), `#+INCLUDE`,
`#+SETUPFILE`, Babel evaluation, `\r\n` line endings (replace them with
`\n` before parsing, which is what Emacs does when it visits the file).

The library builds on Rust 1.75 or later; the CLI follows `clap`'s
requirements.

## License

MIT or Apache-2.0, at your option.
