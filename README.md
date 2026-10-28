# pulldown-org

A pull parser for [Org-mode](https://orgmode.org) written in Rust, in the
style of [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark).
It ships as a library and as a small command-line tool that renders Org to
HTML.

The parser follows the [Org syntax](https://orgmode.org/worg/org-syntax.html)
document and, where the two differ, the reference implementation in Emacs
(`org-element.el`). Its output is tested against Emacs: the test suite compares
the event stream with the parse tree produced by `org-element-parse-buffer` on
a corpus of fixtures.

## Library

```rust
use pulldown_org::{html, Parser};

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
use pulldown_org::{Event, Parser, Tag};

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
the interpretation of `_`/`^`, or to enable inline tasks. In-buffer settings
(`#+TODO:`, `#+OPTIONS: ^:nil`, `#+LINK:`) override the options, as in Emacs.

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
  with the raw body.
- Inside paragraphs `Text` never spans a line; a `SoftBreak` is emitted for
  each newline, like pulldown-cmark.

## Command line

```
pulldown-org README.org            # HTML fragment on stdout
pulldown-org README.org -o out.html --standalone
pulldown-org --events notes.org    # dump the event stream
cat file.org | pulldown-org
```

Install with `cargo install pulldown-org`. The CLI is behind the default
`cli` feature; use `default-features = false` when depending on the library
only.

## Testing against Emacs

`tests/fixtures/*.org` are parsed by Emacs (`tests/emacs/dump.el`) into
`tests/fixtures/*.json` reference trees, checked in so the test suite runs
without Emacs. `sh tools/regen-fixtures.sh` regenerates them; CI checks that
they are up to date with a known Org version. `tools/org-parse.sh 'snippet'`
prints the raw `org-element` tree for any snippet, which is the quickest way
to settle a question about the syntax.

## Status

Early. The whole Org syntax is covered (elements and objects, including
timestamps, footnotes, citations, tables, drawers, in-buffer TODO keywords),
but expect rough edges and API changes before 1.0. Not yet handled: macro
expansion, `#+INCLUDE`, `#+SETUPFILE`, Babel evaluation.

## License

MIT or Apache-2.0, at your option.
