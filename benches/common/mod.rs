//! Inputs shared by the benchmark targets.
//!
//! Every corpus is generated deterministically from a small PRNG, so a
//! number reported by `cargo bench` on one machine is comparable with the
//! same number on another, and nothing large is checked in. Each generator
//! takes a target size in bytes and returns a document of about that size.
//!
//! The corpora fall into three families, see `benches/README.md`:
//!
//! - *documents* that look like real Org files (`notes`, `prose`, `markup`,
//!   `blocks`, `outline`, `fixtures`);
//! - *adversarial* inputs built to expose super-linear behaviour
//!   (`adversarial::*`);
//! - *external* files from `PULLORG_BENCH_CORPUS`, when set, so real
//!   documents (the Org manual, a personal agenda, ...) can be measured
//!   without being committed.

#![allow(dead_code)] // each bench target uses a subset

use std::cell::Cell;
use std::fmt::Write as _;
use std::path::PathBuf;

/// xorshift64, enough for word salad. Interior mutability keeps call sites
/// like `words(rng, out, rng.below(4))` free of borrow juggling.
pub struct Rng(Cell<u64>);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(Cell::new(seed.max(1)))
    }
    pub fn next(&self) -> u64 {
        let mut x = self.0.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0.set(x);
        x
    }
    pub fn below(&self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
    pub fn pick<'a>(&self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const WORDS: &[&str] = &[
    "the",
    "parser",
    "reads",
    "an",
    "Org",
    "document",
    "and",
    "emits",
    "events",
    "for",
    "each",
    "element",
    "object",
    "headline",
    "section",
    "paragraph",
    "with",
    "some",
    "text",
    "that",
    "wraps",
    "over",
    "several",
    "lines",
    "before",
    "a",
    "blank",
    "line",
    "ends",
    "it",
    "while",
    "Emacs",
    "agrees",
    "on",
    "every",
    "byte",
    "range",
    "of",
    "stream",
    "because",
    "reference",
    "implementation",
    "wins",
    "when",
    "syntax",
    "describes",
    "something",
    "else",
];

/// A named input.
pub struct Corpus {
    pub name: &'static str,
    pub text: String,
}

impl Corpus {
    fn new(name: &'static str, text: String) -> Self {
        Corpus { name, text }
    }
}

/// Default size of the document corpora, in bytes.
pub const DEFAULT_SIZE: usize = 64 * 1024;

/// The document corpora, at `size` bytes each.
pub fn documents(size: usize) -> Vec<Corpus> {
    vec![
        Corpus::new("notes", notes(size)),
        Corpus::new("prose", prose(size)),
        Corpus::new("markup", markup(size)),
        Corpus::new("blocks", blocks(size)),
        Corpus::new("outline", outline(size)),
        Corpus::new("fixtures", fixtures(size)),
    ]
}

/// The adversarial corpora, at `size` bytes each.
pub fn adversarial(size: usize) -> Vec<Corpus> {
    adversarial::all(size)
}

/// Files from the directory named by `PULLORG_BENCH_CORPUS` (every `.org`
/// file in it, not recursing), or nothing when the variable is unset.
pub fn external() -> Vec<(String, String)> {
    let Some(dir) = std::env::var_os("PULLORG_BENCH_CORPUS") else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("PULLORG_BENCH_CORPUS={}: {e}", dir.to_string_lossy()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "org"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p).unwrap();
            (name, text)
        })
        .collect()
}

/// Plain-text words wrapped at about 72 columns, no markup characters at
/// all. The floor: how fast the parser scans text it has nothing to do with.
pub fn prose(size: usize) -> String {
    let rng = Rng::new(1);
    let mut out = String::with_capacity(size + 128);
    while out.len() < size {
        paragraph(&rng, &mut out, 3 + rng.below(6), false);
        out.push('\n');
    }
    out
}

/// Paragraphs dense with every inline object: emphasis, code, links of all
/// forms, entities, LaTeX fragments, timestamps, footnote references,
/// sub/superscripts, macros, export snippets, targets, statistics cookies.
pub fn markup(size: usize) -> String {
    let rng = Rng::new(2);
    let mut out = String::with_capacity(size + 128);
    while out.len() < size {
        paragraph(&rng, &mut out, 3 + rng.below(6), true);
        out.push('\n');
    }
    out
}

/// A realistic notes file: an outline three to four levels deep with TODO
/// keywords, priorities, tags, planning lines and property drawers, whose
/// sections hold lightly marked-up paragraphs, lists, the odd table, source
/// block or quote. The number to look at first.
pub fn notes(size: usize) -> String {
    let rng = Rng::new(3);
    let mut out = String::with_capacity(size + 512);
    out.push_str("#+TITLE: Benchmark notes\n#+AUTHOR: pullorg\n#+OPTIONS: toc:nil ^:{}\n#+TODO: TODO NEXT WAIT | DONE CANCELLED\n\n");
    paragraph(&rng, &mut out, 4, true);
    out.push('\n');
    let mut level = 1;
    let mut n = 0;
    while out.len() < size {
        n += 1;
        // Move around the outline: mostly siblings, sometimes deeper or back up.
        match rng.below(10) {
            0..=1 if level < 4 => level += 1,
            2..=3 if level > 1 => level -= 1,
            4 => level = 1,
            _ => {}
        }
        headline(&rng, &mut out, level, n);
        if rng.chance(3) {
            out.push_str(match rng.below(3) {
                0 => "SCHEDULED: <2026-03-14 Sat>\n",
                1 => "DEADLINE: <2026-04-01 Wed +1w>\n",
                _ => "CLOSED: [2026-02-28 Sat 17:03] SCHEDULED: <2026-02-27 Fri>\n",
            });
        }
        if rng.chance(3) {
            let _ = write!(
                out,
                ":PROPERTIES:\n:ID:       bench-{n:05}\n:CREATED:  [2026-01-{:02} Mon 09:00]\n:EFFORT:   {}:00\n:END:\n",
                1 + rng.below(28),
                1 + rng.below(8)
            );
        }
        // Section body: one to four elements.
        for _ in 0..1 + rng.below(4) {
            match rng.below(12) {
                0..=5 => paragraph(&rng, &mut out, 2 + rng.below(5), true),
                6..=7 => list(&rng, &mut out, 2 + rng.below(5), rng.chance(2)),
                8 => table(&rng, &mut out, 3 + rng.below(6), 3),
                9 => src_block(&rng, &mut out, 3 + rng.below(8)),
                10 => quote(&rng, &mut out),
                _ => {
                    out.push_str("#+begin_example\n");
                    for _ in 0..2 + rng.below(4) {
                        words(&rng, &mut out, 8);
                        out.push('\n');
                    }
                    out.push_str("#+end_example\n");
                }
            }
            out.push('\n');
        }
    }
    out
}

/// Element-heavy: nested lists, long tables, source and example blocks,
/// drawers, dynamic blocks, footnote definitions, LaTeX environments,
/// affiliated keywords, comments, fixed-width, rules. Few objects.
pub fn blocks(size: usize) -> String {
    let rng = Rng::new(4);
    let mut out = String::with_capacity(size + 512);
    let mut n = 0;
    while out.len() < size {
        n += 1;
        match n % 11 {
            0 => list(&rng, &mut out, 4 + rng.below(6), true),
            1 => {
                let _ = writeln!(out, "#+NAME: tab{n}\n#+CAPTION: Table {n}");
                table(&rng, &mut out, 8 + rng.below(12), 2 + rng.below(4));
            }
            2 => src_block(&rng, &mut out, 6 + rng.below(20)),
            3 => {
                out.push_str(":LOGBOOK:\nCLOCK: [2026-01-05 Mon 09:00]--[2026-01-05 Mon 10:30] =>  1:30\n- Note taken on [2026-01-05 Mon 10:31] \\\\\n  ");
                words(&rng, &mut out, 6);
                out.push_str("\n:END:\n");
            }
            4 => {
                let _ = writeln!(out, "[fn:{n}] ");
                words(&rng, &mut out, 20);
                out.push('\n');
            }
            5 => out.push_str(
                "\\begin{equation}\n  \\int_0^1 f(x)\\,dx = \\sum_{k=0}^{n} w_k f(x_k)\n\\end{equation}\n",
            ),
            6 => {
                out.push_str("#+BEGIN: clocktable :scope file :maxlevel 2\n| Headline | Time |\n|----------+------|\n| Total    | 1:30 |\n#+END:\n");
            }
            7 => {
                out.push_str("# comment line\n# another\n: fixed width\n: more fixed width\n-----\n");
            }
            8 => {
                out.push_str("#+ATTR_HTML: :class wide\n#+begin_center\n");
                words(&rng, &mut out, 12);
                out.push_str("\n#+end_center\n");
            }
            9 => {
                out.push_str("#+begin_export html\n<div class=\"x\">");
                words(&rng, &mut out, 10);
                out.push_str("</div>\n#+end_export\n");
            }
            _ => quote(&rng, &mut out),
        }
        out.push('\n');
    }
    out
}

/// Only headlines, with every headline feature (keyword, priority, tags,
/// COMMENT, numeric priority, statistics cookie) and levels up to eight.
/// Exercises the headline parser and the tree flattening alone.
pub fn outline(size: usize) -> String {
    let rng = Rng::new(5);
    let mut out = String::with_capacity(size + 128);
    let mut n = 0;
    while out.len() < size {
        n += 1;
        headline(&rng, &mut out, 1 + rng.below(8), n);
    }
    out
}

/// Every `tests/fixtures/*.org`, concatenated and repeated until `size`.
/// Small hand-written documents that together cover the whole syntax.
pub fn fixtures(size: usize) -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("tests/fixtures")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "org"))
        .collect();
    paths.sort();
    let mut one = String::new();
    for p in paths {
        let text = std::fs::read_to_string(&p).unwrap();
        one.push_str(&text);
        if !one.ends_with('\n') {
            one.push('\n');
        }
        one.push('\n');
    }
    let mut out = String::with_capacity(size + one.len());
    while out.len() < size {
        out.push_str(&one);
    }
    out
}

// ---- building blocks ------------------------------------------------------

/// `s.truncate(len)`, backing off to the nearest character boundary.
pub fn truncate_at_char(s: &mut String, mut len: usize) {
    if len >= s.len() {
        return;
    }
    while !s.is_char_boundary(len) {
        len -= 1;
    }
    s.truncate(len);
}

fn words(rng: &Rng, out: &mut String, n: usize) {
    for i in 0..n {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(rng.pick(WORDS));
    }
}

/// One wrapped paragraph followed by a newline; with `rich`, about one word
/// in four is replaced by an inline object.
fn paragraph(rng: &Rng, out: &mut String, lines: usize, rich: bool) {
    for _ in 0..lines {
        let n = 8 + rng.below(6);
        for i in 0..n {
            if i > 0 {
                out.push(' ');
            }
            if rich && rng.chance(4) {
                out.push_str(rng.pick(OBJECTS));
            } else {
                out.push_str(rng.pick(WORDS));
            }
        }
        out.push('\n');
    }
}

const OBJECTS: &[&str] = &[
    "*bold*",
    "/italic/",
    "_underline_",
    "+strike+",
    "=verbatim=",
    "~code~",
    "*nested /emphasis/ here*",
    "[[https://orgmode.org/manual/][the manual]]",
    "[[file:notes.org::*Heading][a heading]]",
    "[[id:0f3a-bench]]",
    "https://example.com/path?q=1",
    "<https://example.com>",
    "\\alpha",
    "\\rarr{}",
    "\\nbsp{}",
    "$x^2 + y^2$",
    "\\(a_i\\)",
    "\\frac{1}{2}",
    "<2026-05-01 Fri>",
    "<2026-05-01 Fri 10:00-11:00>",
    "[2026-05-01 Fri]",
    "<2026-05-01 Fri>--<2026-05-03 Sun>",
    "[fn:1]",
    "[fn::an inline footnote]",
    "x_{i}",
    "e^{i\\pi}",
    "H_2O",
    "{{{title}}}",
    "@@html:<b>raw</b>@@",
    "src_python{print(1)}",
    "<<target>>",
    "[1/3]",
    "[50%]",
    "[cite:@knuth84 p. 3]",
    "call_fn()",
    "a *not bold",
    "x=1",
];

fn headline(rng: &Rng, out: &mut String, level: usize, n: usize) {
    for _ in 0..level {
        out.push('*');
    }
    out.push(' ');
    if rng.chance(3) {
        out.push_str(rng.pick(&["TODO ", "NEXT ", "WAIT ", "DONE ", "CANCELLED "]));
    }
    if rng.chance(5) {
        out.push_str(rng.pick(&["[#A] ", "[#B] ", "[#C] ", "[#3] "]));
    }
    if rng.chance(20) {
        out.push_str("COMMENT ");
    }
    let _ = write!(out, "Heading {n} ");
    words(rng, out, 2 + rng.below(6));
    if rng.chance(6) {
        out.push_str(rng.pick(&[" [1/4]", " [25%]", " [0/0]"]));
    }
    if rng.chance(3) {
        out.push_str(rng.pick(&[
            " :work:",
            " :home:urgent:",
            " :@office:project_x:",
            " :ARCHIVE:",
            "    :a:b:c:d:",
        ]));
    }
    out.push('\n');
}

fn list(rng: &Rng, out: &mut String, items: usize, nested: bool) {
    let ordered = rng.chance(3);
    let descriptive = !ordered && rng.chance(3);
    for i in 0..items {
        let bullet = if ordered {
            format!("{}. ", i + 1)
        } else {
            rng.pick(&["- ", "- ", "+ "]).to_string()
        };
        out.push_str(&bullet);
        if rng.chance(4) {
            out.push_str(rng.pick(&["[ ] ", "[X] ", "[-] "]));
        }
        if descriptive {
            words(rng, out, 1 + rng.below(2));
            out.push_str(" :: ");
        }
        words(rng, out, 3 + rng.below(8));
        out.push('\n');
        if rng.chance(3) {
            out.push_str(&" ".repeat(bullet.len()));
            words(rng, out, 6 + rng.below(6));
            out.push('\n');
        }
        if nested && rng.chance(3) {
            let indent = " ".repeat(bullet.len());
            for j in 0..1 + rng.below(3) {
                let _ = write!(out, "{indent}- ");
                words(rng, out, 2 + rng.below(6));
                out.push('\n');
                if j == 0 && rng.chance(2) {
                    let _ = write!(out, "{indent}  * ");
                    words(rng, out, 2 + rng.below(4));
                    out.push('\n');
                }
            }
        }
    }
}

fn table(rng: &Rng, out: &mut String, rows: usize, cols: usize) {
    let header = |out: &mut String| {
        out.push('|');
        for c in 0..cols {
            let _ = write!(out, " col{c} |");
        }
        out.push('\n');
    };
    header(out);
    out.push('|');
    for _ in 0..cols {
        out.push_str("------+");
    }
    out.pop();
    out.push_str("|\n");
    for r in 0..rows {
        out.push('|');
        for c in 0..cols {
            if c == 0 {
                let _ = write!(out, " {r:>4} |");
            } else if rng.chance(8) {
                let _ = write!(out, " *{}* |", rng.pick(WORDS));
            } else {
                let _ = write!(out, " {:>6} |", rng.below(10_000));
            }
        }
        out.push('\n');
    }
    out.push_str("#+TBLFM: $2=vsum(@I..@II)\n");
}

fn src_block(rng: &Rng, out: &mut String, lines: usize) {
    out.push_str(rng.pick(&[
        "#+begin_src rust\n",
        "#+begin_src emacs-lisp :results silent\n",
        "#+begin_src python :session :exports both\n",
        "#+BEGIN_SRC sh -n -r\n",
    ]));
    for i in 0..lines {
        let indent = "  ".repeat(rng.below(3));
        if rng.chance(12) {
            // Lines that would be syntax outside a block, escaped or not.
            out.push_str(rng.pick(&[",* not a headline\n", ",#+not a keyword\n", "* bare star\n"]));
        } else {
            let _ = write!(out, "{indent}let v{i} = f({}, \"", rng.below(100));
            words(rng, out, 2 + rng.below(4));
            out.push_str("\");\n");
        }
    }
    out.push_str("#+end_src\n");
}

fn quote(rng: &Rng, out: &mut String) {
    out.push_str("#+begin_quote\n");
    paragraph(rng, out, 2 + rng.below(3), true);
    out.push_str("#+end_quote\n");
}

// ---- adversarial ------------------------------------------------------------

/// Inputs built so that a careless implementation goes quadratic, or
/// otherwise far slower per byte than on a normal document. The `scaling`
/// bench runs them at several sizes; their throughput should stay flat.
pub mod adversarial {
    use std::fmt::Write as _;

    use super::Corpus;

    pub fn all(size: usize) -> Vec<Corpus> {
        vec![
            Corpus::new("unclosed-emphasis", repeat("*a /b _c +d =e ~f ", size)),
            Corpus::new("unclosed-links", repeat("[[a [b [fn:c [cite:@d ", size)),
            Corpus::new(
                "backslash-heavy-link",
                format!("[[{}a]]", "\\".repeat(size)),
            ),
            Corpus::new(
                "escaped-brackets-in-link",
                format!("[[{}z]]", repeat("a\\]", size)),
            ),
            Corpus::new("failed-bracket-openers", repeat("[[a\\[", size)),
            Corpus::new("unclosed-latex", repeat("\\( \\[ $$a $b {{{m( @@h: ", size)),
            Corpus::new("emphasis-pairs", repeat("*a* /b/ _c_ +d+ =e= ~f~ ", size)),
            Corpus::new("one-long-line", long_line(size)),
            Corpus::new("many-short-paragraphs", repeat("a\n\n", size)),
            Corpus::new("blank-lines", repeat("x\n\n\n\n\n\n\n\n", size)),
            Corpus::new("deep-headlines", deep_headlines(size)),
            Corpus::new("deep-lists", deep_lists(size)),
            Corpus::new("nested-blocks", nested_blocks(size)),
            Corpus::new(
                "distinct-unterminated-blocks",
                distinct_unterminated_blocks(size),
            ),
            Corpus::new("nested-script-braces", nested_script(size, '{', '}')),
            Corpus::new("nested-script-parens", nested_script(size, '(', ')')),
            Corpus::new("wide-table", wide_table(size)),
            Corpus::new("tall-table", repeat("| a | b | c | d |\n", size)),
            Corpus::new(
                "unterminated-blocks",
                repeat("#+begin_src\n#+begin_quote\n#+begin_x\n", size),
            ),
            Corpus::new(
                "unterminated-drawers",
                repeat(":DRAWER:\n:PROPERTIES:\n:LOGBOOK:\n", size),
            ),
            Corpus::new(
                "unterminated-latex-environments",
                repeat("\\begin{equation}\n", size),
            ),
            Corpus::new(
                "unterminated-blocks-in-list",
                format!("- item\n{}", repeat("  #+begin_x\n", size)),
            ),
            Corpus::new("huge-src-block", huge_src_block(size)),
            Corpus::new("footnote-storm", footnotes(size)),
            Corpus::new("radio-targets", radio(size)),
            Corpus::new(
                "star-lines",
                repeat("*not a headline\n *nor this\n**nor this\n", size),
            ),
            Corpus::new(
                "keyword-lines",
                repeat("#+CAPTION: c\n#+NAME: n\n#+ATTR_HTML: :a b\n", size),
            ),
            Corpus::new("affiliated-orphans", repeat("#+CAPTION: c\n\n", size)),
            Corpus::new(
                "timestamps",
                repeat("<2026-01-01 Thu 10:00-11:00 +1w -2d> ", size),
            ),
            Corpus::new("entities", repeat("\\alpha\\beta\\gamma{}\\delta ", size)),
            Corpus::new(
                "sub-superscripts",
                repeat("a_{b_{c_{d}}} e^{f^{g}} h_i ", size),
            ),
            Corpus::new(
                "non-ascii",
                repeat("héllo wörld — «ĉu» ∀x∈ℝ 日本語 *太字* ", size),
            ),
        ]
    }

    fn repeat(unit: &str, size: usize) -> String {
        let mut out = unit.repeat(size / unit.len() + 1);
        super::truncate_at_char(&mut out, size);
        out
    }

    fn long_line(size: usize) -> String {
        repeat("word *b* /i/ [[l][t]] \\alpha $m$ ", size)
    }

    fn deep_headlines(size: usize) -> String {
        let mut out = String::new();
        let mut level = 1;
        while out.len() < size {
            out.push_str(&"*".repeat(level));
            out.push_str(" h\n");
            level = if level >= 30 { 1 } else { level + 1 };
        }
        out
    }

    fn deep_lists(size: usize) -> String {
        let mut out = String::new();
        while out.len() < size {
            for depth in 0..24 {
                out.push_str(&"  ".repeat(depth));
                out.push_str("- item\n");
            }
        }
        out
    }

    fn wide_table(size: usize) -> String {
        let row = format!("|{}\n", " x |".repeat(200));
        repeat(&row, size)
    }

    fn nested_blocks(size: usize) -> String {
        let depth = (size / 32).max(1);
        let mut out = String::new();
        for i in 0..depth {
            writeln!(out, "#+begin_b{i}").unwrap();
        }
        out.push_str("x\n");
        for i in (0..depth).rev() {
            writeln!(out, "#+end_b{i}").unwrap();
        }
        out
    }

    fn distinct_unterminated_blocks(size: usize) -> String {
        let mut out = String::new();
        let mut i = 0;
        while out.len() < size {
            writeln!(out, "#+begin_missing{i}").unwrap();
            i += 1;
        }
        out
    }

    fn nested_script(size: usize, open: char, close: char) -> String {
        let depth = size.saturating_sub(3) / 2;
        format!(
            "x_{}!{}",
            open.to_string().repeat(depth),
            close.to_string().repeat(depth)
        )
    }

    fn huge_src_block(size: usize) -> String {
        let mut out = String::from("#+begin_src text\n");
        out.push_str(&repeat(
            "  a line of code that is not parsed at all\n",
            size,
        ));
        out.push_str("\n#+end_src\n");
        out
    }

    fn footnotes(size: usize) -> String {
        let mut out = String::new();
        let mut n = 0;
        while out.len() < size {
            n += 1;
            out.push_str(&format!(
                "ref[fn:{n}] and[fn:{n}:inline {n}] and[fn::anon]\n\n[fn:{n}] def {n}\n\n"
            ));
        }
        out
    }

    fn radio(size: usize) -> String {
        // Many radio targets make every word a candidate match.
        let mut out = String::from("<<<the>>> <<<parser>>> <<<and>>> <<<a>>> <<<of>>>\n\n");
        out.push_str(&super::prose(size));
        out
    }
}
