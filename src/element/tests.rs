//! Unit tests for the element parser.
//!
//! Expectations were checked against Emacs' `org-element-parse-buffer`
//! (Org 9.8) with `tools/org-parse.sh`.

use super::*;
use crate::event::{Checkbox, ClockStatus, Timestamp, TimestampKind};
use crate::options::SubSuperscripts;

fn parse(text: &str) -> Document<'_> {
    parse_document(text, &Options::default())
}

/// Compact description of a tree: `kind start..end` per element, children
/// indented.
fn shape(text: &str) -> String {
    let doc = parse(text);
    let mut out = String::new();
    fn go(e: &[Element<'_>], depth: usize, out: &mut String) {
        for el in e {
            out.push_str(&"  ".repeat(depth));
            out.push_str(&format!(
                "{} {}..{}\n",
                kind_name(&el.kind),
                el.range.start,
                el.range.end
            ));
            go(&el.children, depth + 1, out);
        }
    }
    go(&doc.elements, 0, &mut out);
    out
}

fn kind_name(k: &ElementKind<'_>) -> &'static str {
    match k {
        ElementKind::Headline { .. } => "headline",
        ElementKind::Inlinetask { .. } => "inlinetask",
        ElementKind::Section => "section",
        ElementKind::Paragraph { .. } => "paragraph",
        ElementKind::PlainList { .. } => "plain-list",
        ElementKind::Item { .. } => "item",
        ElementKind::Table { .. } => "table",
        ElementKind::TableRow { .. } => "table-row",
        ElementKind::TableCell { .. } => "table-cell",
        ElementKind::CenterBlock => "center-block",
        ElementKind::QuoteBlock => "quote-block",
        ElementKind::VerseBlock { .. } => "verse-block",
        ElementKind::SpecialBlock { .. } => "special-block",
        ElementKind::SrcBlock { .. } => "src-block",
        ElementKind::ExampleBlock { .. } => "example-block",
        ElementKind::ExportBlock { .. } => "export-block",
        ElementKind::CommentBlock { .. } => "comment-block",
        ElementKind::Drawer { .. } => "drawer",
        ElementKind::PropertyDrawer => "property-drawer",
        ElementKind::DynamicBlock { .. } => "dynamic-block",
        ElementKind::FootnoteDefinition { .. } => "footnote-definition",
        ElementKind::Leaf(ev) => match ev {
            Event::Keyword(_) => "keyword",
            Event::Comment(_) => "comment",
            Event::FixedWidth(_) => "fixed-width",
            Event::Rule => "horizontal-rule",
            Event::LatexEnvironment(_) => "latex-environment",
            Event::BabelCall { .. } => "babel-call",
            Event::DiarySexp(_) => "diary-sexp",
            Event::Clock { .. } => "clock",
            Event::Planning { .. } => "planning",
            Event::NodeProperty { .. } => "node-property",
            _ => "leaf?",
        },
    }
}

fn check(text: &str, expected: &str) {
    let got = shape(text);
    let exp: String = expected
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("{}\n", &l[strip_indent(expected)..]))
        .collect();
    assert_eq!(got, exp, "input: {:?}", text);
}

/// Common indentation of the non blank lines of `s`.
fn strip_indent(s: &str) -> usize {
    s.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0)
}

fn leaf<'d, 'a>(e: &'d Element<'a>) -> &'d Event<'a> {
    match &e.kind {
        ElementKind::Leaf(ev) => ev,
        k => panic!("not a leaf: {:?}", k),
    }
}

/// First element of a section.
fn first<'d, 'a>(doc: &'d Document<'a>) -> &'d Element<'a> {
    &doc.elements[0].children[0]
}

#[test]
fn empty_and_blank_documents() {
    assert!(parse("").elements.is_empty());
    assert!(parse("\n\n  \n").elements.is_empty());
    check(
        "\n\n  \nText\n",
        "
        section 5..10
          paragraph 5..10
        ",
    );
}

#[test]
fn no_trailing_newline() {
    let text = "* H\nbody";
    check(
        text,
        "
        headline 0..8
          section 4..8
            paragraph 4..8
        ",
    );
    let doc = parse(text);
    match &doc.elements[0].children[0].children[0].kind {
        ElementKind::Paragraph { content } => assert_eq!(&text[content.clone()], "body"),
        k => panic!("{:?}", k),
    }
}

#[test]
fn paragraph_content_excludes_newline() {
    let text = "line one\nline two\n\n\nnext\n";
    let doc = parse(text);
    let sec = &doc.elements[0];
    assert_eq!(sec.children.len(), 2);
    assert_eq!(sec.children[0].range, 0..18);
    match &sec.children[0].kind {
        ElementKind::Paragraph { content } => {
            assert_eq!(&text[content.clone()], "line one\nline two")
        }
        k => panic!("{:?}", k),
    }
}

#[test]
fn headline_with_every_component() {
    let text = "* TODO [#B] COMMENT My title :a:ARCHIVE:\n";
    check(text, "headline 0..41");
    let doc = parse(text);
    let ElementKind::Headline { data, title } = &doc.elements[0].kind else {
        panic!()
    };
    assert_eq!(data.level, 1);
    assert_eq!(data.todo_keyword.as_deref(), Some("TODO"));
    assert_eq!(data.todo_type, Some(TodoType::Todo));
    assert_eq!(data.priority, Some('B'));
    assert!(data.commented);
    assert!(data.archived);
    assert!(!data.footnote_section);
    assert_eq!(data.tags, vec!["a", "ARCHIVE"]);
    assert_eq!(data.raw_title, "My title");
    assert_eq!(&text[title.clone()], "My title");
}

#[test]
fn headline_edge_cases() {
    // `*` without a space is not a headline; priority may appear after
    // some text (Emacs' `org-priority-regexp` starts with `.*?`).
    let doc = parse("* Foo [#a] bar\n*bold*\n* Footnotes\n* DONE\n* TODOx y\n");
    let hs: Vec<&Headline<'_>> = doc
        .elements
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Headline { data, .. } => Some(data),
            _ => None,
        })
        .collect();
    assert_eq!(hs.len(), 4);
    assert_eq!(hs[0].priority, Some('a'));
    assert_eq!(hs[0].raw_title, "bar");
    assert!(hs[1].footnote_section);
    assert_eq!(hs[2].todo_type, Some(TodoType::Done));
    assert_eq!(hs[2].raw_title, "");
    assert_eq!(hs[3].todo_keyword, None);
    assert_eq!(hs[3].raw_title, "TODOx y");
}

#[test]
fn planning_and_property_drawer() {
    let text = "* H\nSCHEDULED: <2024-01-01 Mon>\n:PROPERTIES:\n:ID: 42\n:EMPTY:\n:END:\nBody\n";
    check(
        text,
        "
        headline 0..72
          section 4..72
            planning 4..32
            property-drawer 32..67
              node-property 45..53
              node-property 53..61
            paragraph 67..72
        ",
    );
    let doc = parse(text);
    let sec = &doc.elements[0].children[0];
    match leaf(&sec.children[0]) {
        Event::Planning {
            scheduled: Some(ts),
            deadline: None,
            closed: None,
        } => {
            assert_eq!(ts.raw, "<2024-01-01 Mon>");
            assert_eq!(ts.kind, TimestampKind::Active);
            assert_eq!(ts.start.unwrap().day, 1);
        }
        e => panic!("{:?}", e),
    }
    let props = &sec.children[1].children;
    assert_eq!(
        leaf(&props[0]),
        &Event::NodeProperty {
            key: "ID".into(),
            value: "42".into()
        }
    );
    assert_eq!(
        leaf(&props[1]),
        &Event::NodeProperty {
            key: "EMPTY".into(),
            value: "".into()
        }
    );
    // Not directly after the headline: a plain drawer and a paragraph.
    let doc = parse("* H\n\nSCHEDULED: <2024-01-01 Mon>\n");
    assert!(matches!(
        doc.elements[0].children[0].children[0].kind,
        ElementKind::Paragraph { .. }
    ));
}

#[test]
fn top_level_property_drawer() {
    check(
        "# c\n:PROPERTIES:\n:ID: top\n:END:\ntext\n",
        "
        section 0..37
          comment 0..4
          property-drawer 4..32
            node-property 17..26
          paragraph 32..37
        ",
    );
}

#[test]
fn nested_lists() {
    let text = "- a\n  - b\n    more b\n  - c\n- d\n";
    check(
        text,
        "
        section 0..31
          plain-list 0..31
            item 0..27
              paragraph 2..4
              plain-list 4..27
                item 4..21
                  paragraph 8..21
                item 21..27
                  paragraph 25..27
            item 27..31
              paragraph 29..31
        ",
    );
    let doc = parse(text);
    let ElementKind::Item { data, tag } = &first(&doc).children[0].kind else {
        panic!()
    };
    assert_eq!(data.bullet, "- ");
    assert!(tag.is_none());
}

#[test]
fn list_ended_by_two_blank_lines() {
    check(
        "- a\n\n- b\n\n\nafter\n",
        "
        section 0..17
          plain-list 0..9
            item 0..4
              paragraph 2..4
            item 5..9
              paragraph 7..9
          paragraph 11..17
        ",
    );
}

#[test]
fn list_indentation_edge_cases() {
    // A less indented line ends the list; a less indented item starts a new
    // list.
    check(
        "  - a\n  text\n - b\nend\n",
        "
        section 0..22
          plain-list 0..6
            item 0..6
              paragraph 4..6
          paragraph 6..13
          plain-list 13..18
            item 13..18
              paragraph 16..18
          paragraph 18..22
        ",
    );
    // `*` bullets need indentation; tabs count as 8 columns.
    let doc = parse("- a\n\tb\n* c\n");
    assert_eq!(doc.elements.len(), 2);
    assert_eq!(doc.elements[0].children[0].range, 0..7);
}

#[test]
fn item_metadata() {
    let text = "1. [@3] [X] three\n2. [-] two\n";
    check(
        text,
        "
        section 0..29
          plain-list 0..29
            item 0..18
              paragraph 12..18
            item 18..29
              paragraph 25..29
        ",
    );
    let doc = parse(text);
    let list = first(&doc);
    assert!(matches!(
        list.kind,
        ElementKind::PlainList {
            kind: ListKind::Ordered
        }
    ));
    let ElementKind::Item { data, .. } = &list.children[0].kind else {
        panic!()
    };
    assert_eq!(data.bullet, "1. ");
    assert_eq!(data.counter.as_deref(), Some("3"));
    assert_eq!(data.checkbox, Some(Checkbox::On));
    let ElementKind::Item { data, .. } = &list.children[1].kind else {
        panic!()
    };
    assert_eq!(data.checkbox, Some(Checkbox::Trans));
}

#[test]
fn descriptive_list() {
    let text = "- term :: def\n- other\n";
    check(
        text,
        "
        section 0..22
          plain-list 0..22
            item 0..14
              paragraph 10..14
            item 14..22
              paragraph 16..22
        ",
    );
    let doc = parse(text);
    let list = first(&doc);
    assert!(matches!(
        list.kind,
        ElementKind::PlainList {
            kind: ListKind::Descriptive
        }
    ));
    let ElementKind::Item { tag: Some(tag), .. } = &list.children[0].kind else {
        panic!()
    };
    assert_eq!(&text[tag.clone()], "term");
    // Only the first item decides the list type.
    let doc = parse("- a\n- b :: c\n");
    assert!(matches!(
        first(&doc).kind,
        ElementKind::PlainList {
            kind: ListKind::Unordered
        }
    ));
}

#[test]
fn paragraph_interruptions() {
    check(
        "a\n- b\nc\n#+BEGIN_X\nd\n:D:\ne\n",
        "
        section 0..26
          paragraph 0..2
          plain-list 2..6
            item 2..6
              paragraph 4..6
          paragraph 6..26
        ",
    );
}

#[test]
fn unclosed_blocks_become_paragraphs() {
    check(
        "#+BEGIN_SRC sh\necho\n\nnext\n",
        "
        section 0..26
          paragraph 0..20
          paragraph 21..26
        ",
    );
    let doc = parse(":NOPE:\ntext\n");
    assert!(matches!(first(&doc).kind, ElementKind::Paragraph { .. }));
}

#[test]
fn blocks() {
    let text = "#+begin_quote\nq\n#+end_quote\n#+BEGIN_VERSE\n v\n#+END_VERSE\n#+BEGIN_aside x\nin\n#+END_aside\n#+BEGIN_SRC python -n :var x=1\n,* a\n#+END_SRC\n#+BEGIN_EXPORT html\n<b/>\n#+END_EXPORT\n#+BEGIN_COMMENT\n,* raw\n#+END_COMMENT\n#+BEGIN_EXAMPLE\n,#+x\n#+END_EXAMPLE\n";
    check(
        text,
        "
        section 0..244
          quote-block 0..28
            paragraph 14..16
          verse-block 28..57
          special-block 57..88
            paragraph 73..76
          src-block 88..134
          export-block 134..172
          comment-block 172..209
          example-block 209..244
        ",
    );
    let doc = parse(text);
    let c = &doc.elements[0].children;
    match &c[1].kind {
        ElementKind::VerseBlock { content } => assert_eq!(&text[content.clone()], " v"),
        k => panic!("{:?}", k),
    }
    assert_eq!(
        c[2].kind,
        ElementKind::SpecialBlock {
            name: "aside".into(),
            parameters: Some("x".into())
        }
    );
    assert_eq!(
        c[3].kind,
        ElementKind::SrcBlock {
            language: Some("python".into()),
            switches: Some("-n".into()),
            parameters: Some(":var x=1".into()),
            value: "* a".into()
        }
    );
    assert_eq!(
        c[4].kind,
        ElementKind::ExportBlock {
            backend: "HTML".into(),
            value: "<b/>".into()
        }
    );
    // Comment blocks are not unescaped (`org-element--substring`).
    assert_eq!(
        c[5].kind,
        ElementKind::CommentBlock {
            value: ",* raw".into()
        }
    );
    assert_eq!(
        c[6].kind,
        ElementKind::ExampleBlock {
            switches: None,
            value: "#+x".into()
        }
    );
    // Values are borrowed when nothing has to be unescaped.
    let doc = parse("#+BEGIN_SRC\n  x\n#+END_SRC\n");
    match &first(&doc).kind {
        ElementKind::SrcBlock { value, .. } => {
            assert!(matches!(value, Cow::Borrowed("  x")))
        }
        k => panic!("{:?}", k),
    }
}

#[test]
fn drawers_and_dynamic_blocks() {
    check(
        ":LOGBOOK:\nnote\n:END:\n:NOPE:\ntext\n",
        "
        section 0..33
          drawer 0..21
            paragraph 10..15
          paragraph 21..33
        ",
    );
    let doc = parse("#+BEGIN: clocktable :maxlevel 2\n#+END:\n");
    assert_eq!(first(&doc).range, 0..39);
    assert_eq!(
        first(&doc).kind,
        ElementKind::DynamicBlock {
            name: "clocktable".into(),
            parameters: Some(":maxlevel 2".into())
        }
    );
}

#[test]
fn footnote_definitions() {
    check(
        "[fn:1] First\nline\n\n[fn:2]\n\nSecond\n\n\nOut\n",
        "
        section 0..40
          footnote-definition 0..18
            paragraph 7..18
          footnote-definition 19..34
            paragraph 27..34
          paragraph 36..40
        ",
    );
    let doc = parse("[fn:x] a\n");
    assert_eq!(
        first(&doc).kind,
        ElementKind::FootnoteDefinition { label: "x".into() }
    );
}

#[test]
fn tables() {
    let text = "| a | b |\n|---+---|\n| c |\n#+TBLFM: $1=1\n";
    check(
        text,
        "
        section 0..40
          table 0..40
            table-row 0..10
              table-cell 1..5
              table-cell 5..9
            table-row 10..20
            table-row 20..26
              table-cell 21..25
        ",
    );
    let doc = parse(text);
    let t = first(&doc);
    let ElementKind::Table {
        kind,
        formulas,
        value,
    } = &t.kind
    else {
        panic!()
    };
    assert_eq!(*kind, TableKind::Org);
    assert_eq!(formulas, &vec![Cow::Borrowed("$1=1")]);
    // Several formula lines: Emacs' order, last line first.
    let doc = parse("| a |\n#+TBLFM: $1=1\n#+TBLFM: $1=2\n");
    match &first(&doc).kind {
        ElementKind::Table { formulas, .. } => assert_eq!(formulas, &vec!["$1=2", "$1=1"]),
        k => panic!("{:?}", k),
    }
    assert!(value.is_none());
    assert_eq!(
        t.children[1].kind,
        ElementKind::TableRow {
            kind: TableRowKind::Rule
        }
    );
    match &t.children[0].children[1].kind {
        ElementKind::TableCell { content } => assert_eq!(&text[content.clone()], "b"),
        k => panic!("{:?}", k),
    }
    let text = "+---+\n| a |\n+---+\n";
    let doc = parse(text);
    match &first(&doc).kind {
        ElementKind::Table {
            kind: TableKind::TableEl,
            value: Some(v),
            ..
        } => assert_eq!(v, text),
        k => panic!("{:?}", k),
    }
}

#[test]
fn leaf_elements() {
    let text = "# a\n# b\n: f1\n: f2\n-----\n%%(diary)\n#+CALL: f(x=1)\nCLOCK: [2024-01-01 Mon 10:00]--[2024-01-01 Mon 11:00] =>  1:00\n\\begin{eq}\nx\n\\end{eq}\n#+TITLE: T\n";
    check(
        text,
        "
        section 0..145
          comment 0..8
          fixed-width 8..18
          horizontal-rule 18..24
          diary-sexp 24..34
          babel-call 34..49
          clock 49..112
          latex-environment 112..134
          keyword 134..145
        ",
    );
    let doc = parse(text);
    let c = &doc.elements[0].children;
    assert_eq!(leaf(&c[0]), &Event::Comment("a\nb".into()));
    assert_eq!(leaf(&c[1]), &Event::FixedWidth("f1\nf2".into()));
    assert_eq!(leaf(&c[3]), &Event::DiarySexp("%%(diary)".into()));
    assert_eq!(
        leaf(&c[4]),
        &Event::BabelCall {
            call: "f".into(),
            inside_header: None,
            arguments: Some("x=1".into()),
            end_header: None
        }
    );
    match leaf(&c[5]) {
        Event::Clock {
            timestamp,
            duration,
            status,
        } => {
            assert_eq!(timestamp.kind, TimestampKind::InactiveRange);
            assert_eq!(duration.as_deref(), Some("1:00"));
            assert_eq!(*status, ClockStatus::Closed);
        }
        e => panic!("{:?}", e),
    }
    assert_eq!(
        leaf(&c[6]),
        &Event::LatexEnvironment("\\begin{eq}\nx\n\\end{eq}\n".into())
    );
    assert_eq!(
        leaf(&c[7]),
        &Event::Keyword(Keyword {
            key: "TITLE".into(),
            value: "T".into(),
            optional: None,
            affiliated: false
        })
    );
}

#[test]
fn affiliated_keywords() {
    let text = "#+NAME: p\n#+CAPTION[s]: long\n#+attr_html: :w 1\n#+tblname: t\nPara.\n";
    let doc = parse(text);
    let p = first(&doc);
    assert!(matches!(p.kind, ElementKind::Paragraph { .. }));
    assert_eq!(p.range, 0..66);
    let kws: Vec<_> = p
        .affiliated
        .iter()
        .map(|(k, r)| {
            (
                &*k.key,
                &*k.value,
                k.optional.as_deref(),
                k.affiliated,
                r.clone(),
            )
        })
        .collect();
    assert_eq!(
        kws,
        vec![
            ("NAME", "p", None, true, 0..10),
            ("CAPTION", "long", Some("s"), true, 10..29),
            ("ATTR_HTML", ":w 1", None, true, 29..47),
            ("NAME", "t", None, true, 47..60),
        ]
    );
    // Before a headline, a blank line or at the end: plain keywords.
    check(
        "#+NAME: n\n* H\n",
        "
        section 0..10
          keyword 0..10
        headline 10..14
        ",
    );
    check(
        "#+NAME: n",
        "
        section 0..9
          keyword 0..9
        ",
    );
    let doc = parse("#+NAME: n\n\npara\n");
    assert!(first(&doc).affiliated.is_empty());
    assert!(matches!(
        leaf(first(&doc)),
        Event::Keyword(Keyword {
            affiliated: false,
            ..
        })
    ));
    // Affiliated keywords may attach to a keyword.
    let doc = parse("#+NAME: n\n#+TITLE: t\n");
    assert_eq!(first(&doc).affiliated.len(), 1);
}

#[test]
fn in_buffer_settings() {
    let text = "#+TODO: TODO(t) NEXT | DONE(d!) CANCELED\n#+SEQ_TODO: A B\n#+OPTIONS: toc:nil ^:{}\n#+LINK: gh https://github.com/%s\n* NEXT task\n* B b\n* CANCELED c\n* <<<radio>>>\n";
    let doc = parse(text);
    let s = &doc.settings;
    assert_eq!(s.todo_keywords, vec!["TODO", "NEXT", "A"]);
    assert_eq!(s.done_keywords, vec!["DONE", "CANCELED", "B"]);
    assert_eq!(s.sub_superscripts, SubSuperscripts::Braces);
    assert_eq!(
        s.link_abbreviations,
        vec![(Cow::Borrowed("gh"), Cow::Borrowed("https://github.com/%s"))]
    );
    assert_eq!(s.radio_targets, vec!["radio"]);
    let types: Vec<_> = doc
        .elements
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Headline { data, .. } => Some(data.todo_type),
            _ => None,
        })
        .collect();
    assert_eq!(
        types,
        vec![
            Some(TodoType::Todo),
            Some(TodoType::Done),
            Some(TodoType::Done),
            None
        ]
    );
    // Without in-buffer keywords, the options apply.
    let opts = Options {
        todo_keywords: vec!["WAIT".into()],
        ..Options::default()
    };
    let doc = parse_document("* WAIT x\n* TODO y\n", &opts);
    let kws: Vec<_> = doc
        .elements
        .iter()
        .map(|e| match &e.kind {
            ElementKind::Headline { data, .. } => data.todo_keyword.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(kws, vec![Some("WAIT".into()), None]);
    // A keyword inside a block is not a setting.
    let doc = parse("#+BEGIN_SRC\n#+TODO: X\n#+END_SRC\n* X y\n");
    assert_eq!(doc.settings.todo_keywords, vec!["TODO"]);
}

#[test]
fn inline_tasks() {
    let text = "* H\n*************** TODO task\nbody\n*************** END\nafter\n";
    // Disabled by default: plain headlines.
    check(
        text,
        "
        headline 0..61
          headline 4..35
            section 30..35
              paragraph 30..35
          headline 35..61
            section 55..61
              paragraph 55..61
        ",
    );
    let opts = Options {
        inlinetask_min_level: Some(15),
        ..Options::default()
    };
    let doc = parse_document(text, &opts);
    let sec = &doc.elements[0].children[0];
    assert_eq!(sec.range, 4..61);
    let task = &sec.children[0];
    assert_eq!(task.range, 4..55);
    let ElementKind::Inlinetask { data, title } = &task.kind else {
        panic!("{:?}", task.kind)
    };
    assert_eq!(data.level, 15);
    assert_eq!(data.todo_keyword.as_deref(), Some("TODO"));
    assert_eq!(&text[title.clone()], "task");
    assert_eq!(task.children.len(), 1);
    assert_eq!(task.children[0].range, 30..35);
    assert_eq!(sec.children[1].range, 55..61);
}

#[test]
fn unicode_input() {
    let text = "* Été :é:\nPàra ü\n| é |\n";
    check(
        text,
        "
        headline 0..29
          section 13..29
            paragraph 13..22
            table 22..29
              table-row 22..29
                table-cell 23..28
        ",
    );
    let doc = parse(text);
    let ElementKind::Headline { data, .. } = &doc.elements[0].kind else {
        panic!()
    };
    assert_eq!(data.tags, vec!["é"]);
    assert_eq!(data.raw_title, "Été");
}

#[test]
fn escaping_commas() {
    assert_eq!(
        parse::unescape(",* a\n  ,,#+b\n,#c\n ,*\n,x"),
        "* a\n  ,#+b\n,#c\n *\n,x"
    );
}

#[test]
fn crlf_is_not_normalised() {
    // `\r` stays part of the line; ranges remain valid byte offsets.
    let text = "* H\r\nline\r\n";
    let doc = parse(text);
    assert_eq!(doc.elements[0].range, 0..text.len());
}

// ---------------------------------------------------------------------------
// Differential dump, mirroring the Elisp dumper used during development:
// `ORG_DUMP_FILE=x.org cargo test --lib element::tests::dump -- --ignored
// --nocapture`.

fn q(s: Option<&str>) -> String {
    match s {
        None => "nil".into(),
        Some(s) => {
            let mut o = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    '\n' => o.push_str("\\n"),
                    '\x0c' => o.push_str("\\f"),
                    c => o.push(c),
                }
            }
            o.push('"');
            o
        }
    }
}

fn tf(b: bool) -> &'static str {
    if b {
        "t"
    } else {
        "nil"
    }
}

fn ts(t: &Option<Timestamp<'_>>) -> String {
    q(t.as_ref().map(|t| &*t.raw))
}

fn dump_tree(text: &str, els: &[Element<'_>], depth: usize, out: &mut Vec<String>) {
    for el in els {
        let pa = match &el.kind {
            ElementKind::TableCell { .. } => "nil".to_string(),
            _ => el
                .affiliated
                .last()
                .map_or(el.range.start, |(_, r)| r.end)
                .to_string(),
        };
        let extra = match &el.kind {
            ElementKind::Headline { data, .. } => format!(
                "lvl={} todo={} prio={} tags={} com={} arch={} fns={} raw={}",
                data.level,
                q(data.todo_keyword.as_deref()),
                data.priority.map_or("nil".to_string(), |c| match c {
                    '0'..='9' => (c as u32 - '0' as u32).to_string(),
                    c => (c as u32).to_string(),
                }),
                q(Some(&data.tags.join(":"))),
                tf(data.commented),
                tf(data.archived),
                tf(data.footnote_section),
                q(Some(&data.raw_title))
            ),
            ElementKind::Inlinetask { data, .. } => format!(
                "lvl={} todo={} raw={}",
                data.level,
                q(data.todo_keyword.as_deref()),
                q(Some(&data.raw_title))
            ),
            ElementKind::Paragraph { content } | ElementKind::VerseBlock { content } => {
                format!("c={}..{}", content.start, content.end)
            }
            ElementKind::PlainList { kind } => format!(
                "type={}",
                match kind {
                    ListKind::Ordered => "ordered",
                    ListKind::Unordered => "unordered",
                    ListKind::Descriptive => "descriptive",
                }
            ),
            ElementKind::Item { data, tag } => format!(
                "bullet={} cb={} counter={} tag={}",
                q(Some(&data.bullet)),
                match data.checkbox {
                    None => "nil",
                    Some(Checkbox::On) => "on",
                    Some(Checkbox::Off) => "off",
                    Some(Checkbox::Trans) => "trans",
                },
                data.counter.as_deref().map_or("nil".to_string(), |c| {
                    c.parse::<u32>().map(|n| n.to_string()).unwrap_or_else(|_| {
                        (c.to_ascii_uppercase().chars().next().unwrap() as u32 - 64).to_string()
                    })
                }),
                q(tag.as_ref().map(|t| &text[t.clone()]))
            ),
            ElementKind::SrcBlock {
                language,
                switches,
                parameters,
                value,
            } => format!(
                "lang={} sw={} params={} value={}",
                q(language.as_deref()),
                q(switches.as_deref()),
                q(parameters.as_deref()),
                q(Some(value))
            ),
            ElementKind::ExampleBlock { switches, value } => {
                format!("sw={} value={}", q(switches.as_deref()), q(Some(value)))
            }
            ElementKind::ExportBlock { backend, value } => format!(
                "type={} value={}",
                q(if backend.is_empty() {
                    None
                } else {
                    Some(backend)
                }),
                q(Some(value))
            ),
            ElementKind::CommentBlock { value } => format!("value={}", q(Some(value))),
            ElementKind::Table {
                kind,
                formulas,
                value,
            } => format!(
                "type={} tblfm={} value={}",
                match kind {
                    TableKind::Org => "org",
                    TableKind::TableEl => "table.el",
                },
                q(Some(&formulas.join("|"))),
                q(value.as_deref())
            ),
            ElementKind::TableRow { kind } => format!(
                "type={}",
                match kind {
                    TableRowKind::Standard => "standard",
                    TableRowKind::Rule => "rule",
                }
            ),
            ElementKind::TableCell { content } => format!("c={}..{}", content.start, content.end),
            ElementKind::Drawer { name } => format!("name={}", q(Some(name))),
            ElementKind::SpecialBlock { name, parameters } => {
                format!("type={} params={}", q(Some(name)), q(parameters.as_deref()))
            }
            ElementKind::DynamicBlock { name, parameters } => {
                format!("name={} args={}", q(Some(name)), q(parameters.as_deref()))
            }
            ElementKind::FootnoteDefinition { label } => format!("label={}", q(Some(label))),
            ElementKind::Leaf(ev) => match ev {
                Event::Keyword(k) => format!("key={} value={}", q(Some(&k.key)), q(Some(&k.value))),
                Event::Comment(v)
                | Event::FixedWidth(v)
                | Event::LatexEnvironment(v)
                | Event::DiarySexp(v) => {
                    format!("value={}", q(Some(v)))
                }
                Event::NodeProperty { key, value } => {
                    format!("key={} value={}", q(Some(key)), q(Some(value)))
                }
                Event::Clock {
                    timestamp,
                    duration,
                    status,
                } => format!(
                    "status={} dur={} ts={}",
                    match status {
                        ClockStatus::Closed => "closed",
                        ClockStatus::Running => "running",
                    },
                    q(duration.as_deref()),
                    if timestamp.raw.is_empty() {
                        "nil".to_string()
                    } else {
                        q(Some(&timestamp.raw))
                    }
                ),
                Event::Planning {
                    closed,
                    scheduled,
                    deadline,
                } => format!(
                    "closed={} sched={} dead={}",
                    ts(closed),
                    ts(scheduled),
                    ts(deadline)
                ),
                Event::BabelCall {
                    call,
                    inside_header,
                    arguments,
                    end_header,
                } => format!(
                    "call={} in={} args={} end={}",
                    q(if call.is_empty() { None } else { Some(call) }),
                    q(inside_header.as_deref()),
                    q(arguments.as_deref()),
                    q(end_header.as_deref())
                ),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let mut extra = extra;
        if !el.affiliated.is_empty() {
            let mut keys: Vec<String> = el
                .affiliated
                .iter()
                .map(|(k, _)| k.key.to_lowercase())
                .collect();
            keys.sort();
            keys.dedup();
            let name = el
                .affiliated
                .iter()
                .rev()
                .find(|(k, _)| k.key == "NAME")
                .map(|(k, _)| &*k.value);
            extra = format!("{} aff={} name={}", extra, keys.join(","), q(name));
        }
        out.push(format!(
            "{}{} {}..{} pa={} {}",
            "  ".repeat(depth),
            kind_name(&el.kind),
            el.range.start,
            el.range.end,
            pa,
            extra
        ));
        dump_tree(text, &el.children, depth + 1, out);
    }
}

#[test]
#[ignore]
fn dump() {
    let paths = std::env::var("ORG_DUMP_FILE").expect("ORG_DUMP_FILE");
    println!("BEGIN-DUMP");
    for path in paths.split(',') {
        let text = std::fs::read_to_string(path).unwrap();
        let mut opts = Options::default();
        if let Ok(n) = std::env::var("ORG_DUMP_INLINE") {
            opts.inlinetask_min_level = n.parse().ok();
        }
        let doc = parse_document(&text, &opts);
        let mut out = Vec::new();
        dump_tree(&text, &doc.elements, 0, &mut out);
        let s = &doc.settings;
        let j =
            |v: &Vec<CowStr<'_>>, sep: &str| v.iter().map(|x| &**x).collect::<Vec<_>>().join(sep);
        println!(
            "=== {}",
            std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
        );
        for l in out {
            println!("{}", l);
        }
        println!(
            "SETTINGS todo={} done={} radio={}",
            q(Some(&j(&s.todo_keywords, " "))),
            q(Some(&j(&s.done_keywords, " "))),
            q(Some(&j(&s.radio_targets, "|")))
        );
    }
    println!("END-DUMP");
}
