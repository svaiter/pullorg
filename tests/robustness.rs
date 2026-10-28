//! Random-input robustness: the parser must never panic, must emit balanced
//! `Start`/`End` pairs, and the HTML renderer must accept any stream.

use std::ops::Range;

use pullorg::{html, Event, Options, Parser, SubSuperscripts, TagEnd};

/// Small xorshift PRNG so the test is deterministic without dependencies.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const TOKENS: &[&str] = &[
    "* ",
    "** ",
    "*** ",
    "TODO ",
    "DONE ",
    "[#A] ",
    "[#10] ",
    ":tag:",
    ":a:b:",
    "COMMENT ",
    "\n",
    "\n\n",
    "\n\n\n",
    " ",
    "  ",
    "\t",
    "- ",
    "+ ",
    "1. ",
    "2) ",
    "[ ] ",
    "[X] ",
    "[@3] ",
    " :: ",
    "| a | b |",
    "|---+---|",
    "|",
    "#+TBLFM: $1=2",
    "#+BEGIN_SRC rust",
    "#+END_SRC",
    "#+begin_quote",
    "#+end_quote",
    "#+BEGIN_EXAMPLE",
    "#+END_EXAMPLE",
    "#+BEGIN_EXPORT html",
    "#+END_EXPORT",
    "#+BEGIN: clocktable",
    "#+END:",
    "#+BEGIN_foo",
    "#+END_foo",
    "#+NAME: x",
    "#+CAPTION: cap",
    "#+ATTR_HTML: :class c",
    "#+TODO: TODO NEXT | DONE",
    "#+OPTIONS: ^:nil",
    "#+LINK: gh https://github.com/%s",
    ":PROPERTIES:",
    ":END:",
    ":ID: 1",
    ":DRAWER:",
    "SCHEDULED: <2024-01-01 Mon>",
    "DEADLINE: <2024-01-02 Tue +1w>",
    "CLOSED: [2024-01-03 Wed 10:00]",
    "CLOCK: [2024-01-01 Mon 10:00]--[2024-01-01 Mon 11:00] =>  1:00",
    "[fn:1] def",
    "[fn:1]",
    "[fn::inline]",
    "[fn:2:lab]",
    "-----",
    "# comment",
    ": fixed",
    "%%(diary-float t 4 2)",
    "#+CALL: f(1)",
    "\\begin{equation}",
    "\\end{equation}",
    "*bold*",
    "/it/",
    "_ul_",
    "+st+",
    "~code~",
    "=verb=",
    "*",
    "/",
    "_",
    "+",
    "~",
    "=",
    "\\alpha",
    "\\alpha{}",
    "\\_ ",
    "$x$",
    "$$x$$",
    "\\(x\\)",
    "\\[x\\]",
    "\\command{a}",
    "@@html:<b>@@",
    "[[https://a.b][d]]",
    "[[file:a.png]]",
    "[[*Head]]",
    "[[#id]]",
    "[[(ref)]]",
    "[[gh:a/b]]",
    "<https://x.y>",
    "https://x.y/z",
    "<<target>>",
    "<<<radio>>>",
    "radio",
    "{{{m(a,b)}}}",
    "[33%]",
    "[1/3]",
    "a_b",
    "a^{b}",
    "a_{b",
    "x^*",
    "<2024-01-01 Mon>",
    "[2024-01-01 Mon 10:00-11:00]",
    "<2024-01-01 Mon>--<2024-01-02 Tue>",
    "<%%(diary-float t 4 2)>",
    "src_rust{let x = 1;}",
    "call_f[:x](1)[:y]",
    "[cite/t:@k1; see @k2 p. 3]",
    "\\\\",
    "\\\\\n",
    "[",
    "]",
    "[[",
    "]]",
    "<",
    ">",
    "{",
    "}",
    "(",
    ")",
    "\\",
    "$",
    "@",
    "#",
    "é",
    "中文",
    "日本語",
    "😀",
    "\u{200b}",
    "\u{a0}",
    "text",
    "word",
    "A",
    ",*",
    ",#+",
    "\r",
];

fn random_document(rng: &mut Rng) -> String {
    let len = 1 + rng.below(80);
    let mut s = String::new();
    for _ in 0..len {
        s.push_str(rng.pick(TOKENS));
    }
    s
}

fn check(text: &str, options: Options) {
    let events: Vec<(Event, Range<usize>)> = Parser::new_ext(text, options.clone())
        .into_offset_iter()
        .collect();
    let mut stack: Vec<TagEnd> = Vec::new();
    // Raw containers hold their whole body in a single `Text`.
    let raw = |end: &TagEnd| {
        matches!(
            end,
            TagEnd::SrcBlock
                | TagEnd::ExampleBlock
                | TagEnd::ExportBlock
                | TagEnd::CommentBlock
                | TagEnd::Table
        )
    };
    for (event, range) in &events {
        assert!(
            range.start <= range.end && range.end <= text.len(),
            "bad range {range:?} for {event:?} in {text:?}"
        );
        assert!(
            text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
            "range {range:?} splits a char in {text:?}"
        );
        match event {
            Event::Start(tag) => stack.push(tag.to_end()),
            Event::End(end) => {
                let open = stack
                    .pop()
                    .unwrap_or_else(|| panic!("End({end:?}) without Start in {text:?}"));
                assert_eq!(open, *end, "mismatched End in {text:?}");
            }
            Event::Text(t) if !stack.iter().any(raw) => {
                assert!(!t.contains('\n'), "Text with newline in {text:?}")
            }
            _ => {}
        }
    }
    assert!(stack.is_empty(), "unclosed {stack:?} in {text:?}");
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(text, options));
}

#[test]
fn random_documents_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let iterations = std::env::var("PULLORG_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000);
    for i in 0..iterations {
        let text = random_document(&mut rng);
        let mut options = Options::default();
        match i % 4 {
            1 => options.sub_superscripts = SubSuperscripts::Braces,
            2 => options.sub_superscripts = SubSuperscripts::Never,
            3 => options.inlinetask_min_level = Some(3),
            _ => {}
        }
        check(&text, options);
    }
}

#[test]
fn every_prefix_and_suffix_of_fixtures_parses() {
    // Truncated documents exercise every "unexpected end of input" path.
    for entry in std::fs::read_dir("tests/fixtures").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("org") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let step = (text.len() / 40).max(1);
        let mut cut = 0;
        while cut <= text.len() {
            if text.is_char_boundary(cut) {
                check(&text[..cut], Options::default());
                check(&text[cut..], Options::default());
            }
            cut += step;
        }
    }
}
