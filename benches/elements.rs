//! Element-level microbenchmarks: documents made of one element kind each,
//! so a regression in one parser shows up under its own name rather than
//! diluted in a full document. `elements/<kind>` parses about 16 KiB of
//! that element and drains the stream (objects included, as a user would).

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use pullorg::Parser;

const SIZE: usize = 16 * 1024;

fn repeat_to(unit: &str, size: usize) -> String {
    unit.repeat(size / unit.len() + 1)
}

fn inputs() -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = vec![
        ("headline-plain", repeat_to("* A heading with a few words\n", SIZE)),
        (
            "headline-full",
            repeat_to("*** TODO [#A] COMMENT A heading [1/3] :tag1:tag2:\n", SIZE),
        ),
        ("headline-deep", repeat_to("******** deep heading\n", SIZE)),
        (
            "headline-section",
            repeat_to("** TODO Task\nSCHEDULED: <2026-01-01 Thu>\n:PROPERTIES:\n:ID: x\n:END:\nBody text.\n\n", SIZE),
        ),
        ("paragraph", repeat_to("A paragraph of plain words that\nwraps over two lines.\n\n", SIZE)),
        ("list-flat", repeat_to("- item one\n- item two\n- item three\n", SIZE)),
        (
            "list-nested",
            repeat_to("- a\n  - b\n    - c\n      - d\n  - e\n- f\n", SIZE),
        ),
        (
            "list-ordered-checkbox",
            repeat_to("1. [X] done\n2. [ ] todo [1/2]\n3. [-] partly\n", SIZE),
        ),
        ("list-descriptive", repeat_to("- term :: definition text\n", SIZE)),
        (
            "table",
            repeat_to("| a | b | c |\n|---+---+---|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n#+TBLFM: $3=$1+$2\n\n", SIZE),
        ),
        (
            "table-el",
            repeat_to("+---+---+\n| a | b |\n+---+---+\n\n", SIZE),
        ),
        (
            "src-block",
            repeat_to("#+begin_src rust :tangle yes\nfn f() {}\nlet x = 1;\n#+end_src\n\n", SIZE),
        ),
        (
            "src-block-escaped",
            repeat_to("#+begin_src org\n,* heading\n,#+keyword\n#+end_src\n\n", SIZE),
        ),
        ("example-block", repeat_to("#+begin_example\nline\nline\n#+end_example\n\n", SIZE)),
        ("quote-block", repeat_to("#+begin_quote\nQuoted words here.\n#+end_quote\n\n", SIZE)),
        ("special-block", repeat_to("#+begin_note :x y\nWords.\n#+end_note\n\n", SIZE)),
        ("export-block", repeat_to("#+begin_export html\n<p>x</p>\n#+end_export\n\n", SIZE)),
        ("drawer", repeat_to(":NOTES:\nSome words.\n:END:\n\n", SIZE)),
        (
            "property-drawer",
            repeat_to("* H\n:PROPERTIES:\n:A: 1\n:B: 2\n:C+: 3\n:END:\n", SIZE),
        ),
        (
            "planning",
            repeat_to("* H\nSCHEDULED: <2026-01-01 Thu> DEADLINE: <2026-01-02 Fri +1w> CLOSED: [2026-01-03 Sat]\n", SIZE),
        ),
        (
            "clock",
            repeat_to("CLOCK: [2026-01-01 Thu 10:00]--[2026-01-01 Thu 11:00] =>  1:00\n", SIZE),
        ),
        ("keyword", repeat_to("#+KEY: value\n", SIZE)),
        ("keyword-todo", repeat_to("#+TODO: A B | C\n", SIZE)),
        (
            "affiliated",
            repeat_to("#+NAME: n\n#+CAPTION[s]: c\n#+ATTR_HTML: :a b\nPara.\n\n", SIZE),
        ),
        ("footnote-def", repeat_to("[fn:1] The definition.\n\n", SIZE)),
        ("latex-env", repeat_to("\\begin{equation}\nx = 1\n\\end{equation}\n\n", SIZE)),
        ("fixed-width", repeat_to(": fixed\n: width\n", SIZE)),
        ("comment", repeat_to("# a comment\n# another\n", SIZE)),
        ("horizontal-rule", repeat_to("-----\n\n", SIZE)),
        ("dynamic-block", repeat_to("#+BEGIN: name :p 1\nbody\n#+END:\n\n", SIZE)),
        ("babel-call", repeat_to("#+CALL: f(x=1)\n", SIZE)),
        ("diary-sexp", repeat_to("%%(diary-anniversary 1 1 2000) x\n", SIZE)),
    ];
    for (_, text) in &mut v {
        common::truncate_at_char(text, SIZE);
        // Cut at a line so the tail is not a half element.
        if let Some(i) = text.rfind('\n') {
            text.truncate(i + 1);
        }
    }
    v
}

fn elements(c: &mut Criterion) {
    let mut group = c.benchmark_group("elements");
    for (name, text) in inputs() {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &text, |b, text| {
            b.iter(|| Parser::new(black_box(text)).count())
        });
    }
    group.finish();
}

/// Shorter than Criterion's defaults; the inputs are small and the suite is
/// large. Command-line flags (`--warm-up-time`, `--measurement-time`,
/// `--quick`) still override these.
fn config() -> Criterion {
    Criterion::default()
        .warm_up_time(Duration::from_millis(300))
        .measurement_time(Duration::from_secs(2))
}

criterion_group! {
    name = benches;
    config = config();
    targets = elements
}
criterion_main!(benches);
