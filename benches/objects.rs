//! Object-level microbenchmarks through `Parser::new_inline`, which parses
//! a string as a secondary string: no elements, only objects. Each
//! `objects/<kind>` input is about 8 KiB of one object kind separated by
//! words, so the number is the cost of recognising that object, not of the
//! paragraph around it. `objects/options-*` vary the parser options that
//! change the object grammar.

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use pullorg::{Options, Parser, SubSuperscripts};

const SIZE: usize = 8 * 1024;

fn repeat_to(unit: &str, size: usize) -> String {
    let mut s = unit.repeat(size / unit.len() + 1);
    common::truncate_at_char(&mut s, size);
    // End on a whole unit so the last object is well formed.
    if let Some(i) = s.rfind(unit) {
        s.truncate(i + unit.len());
    }
    s
}

fn inputs() -> Vec<(&'static str, String)> {
    let w = "some plain words between ";
    let unit = |obj: &str| format!("{w}{obj} ");
    vec![
        ("text-only", repeat_to(w, SIZE)),
        (
            "text-with-markers",
            repeat_to("a * b / c _ d + e = f ~ g [ h < i $ j \\ k ^ l ", SIZE),
        ),
        ("bold", repeat_to(&unit("*bold*"), SIZE)),
        ("italic", repeat_to(&unit("/italic/"), SIZE)),
        ("underline", repeat_to(&unit("_under_"), SIZE)),
        ("strike", repeat_to(&unit("+strike+"), SIZE)),
        ("code", repeat_to(&unit("~code~"), SIZE)),
        ("verbatim", repeat_to(&unit("=verbatim="), SIZE)),
        (
            "emphasis-nested",
            repeat_to(&unit("*bold /italic _under_/*"), SIZE),
        ),
        (
            "link-bracket",
            repeat_to(&unit("[[https://example.org/a/b][desc]]"), SIZE),
        ),
        (
            "link-bracket-fuzzy",
            repeat_to(&unit("[[Some heading]]"), SIZE),
        ),
        (
            "link-plain",
            repeat_to(&unit("https://example.org/a/b?c=d"), SIZE),
        ),
        (
            "link-angle",
            repeat_to(&unit("<https://example.org/a>"), SIZE),
        ),
        ("link-abbrev", repeat_to(&unit("[[gh:owner/repo]]"), SIZE)),
        ("entity", repeat_to(&unit("\\alpha"), SIZE)),
        ("entity-braces", repeat_to(&unit("\\rarr{}"), SIZE)),
        ("latex-dollar", repeat_to(&unit("$x^2+y^2$"), SIZE)),
        ("latex-paren", repeat_to(&unit("\\(a+b\\)"), SIZE)),
        ("latex-command", repeat_to(&unit("\\frac{1}{2}"), SIZE)),
        (
            "timestamp-active",
            repeat_to(&unit("<2026-01-01 Thu>"), SIZE),
        ),
        (
            "timestamp-full",
            repeat_to(&unit("<2026-01-01 Thu 10:00-11:00 +1w -2d>"), SIZE),
        ),
        (
            "timestamp-range",
            repeat_to(&unit("<2026-01-01 Thu>--<2026-01-02 Fri>"), SIZE),
        ),
        (
            "timestamp-inactive",
            repeat_to(&unit("[2026-01-01 Thu]"), SIZE),
        ),
        ("footnote-ref", repeat_to(&unit("[fn:1]"), SIZE)),
        (
            "footnote-inline",
            repeat_to(&unit("[fn::inline text]"), SIZE),
        ),
        ("subscript", repeat_to(&unit("x_{i}"), SIZE)),
        ("subscript-bare", repeat_to(&unit("x_i"), SIZE)),
        ("superscript", repeat_to(&unit("e^{2}"), SIZE)),
        ("macro", repeat_to(&unit("{{{title}}}"), SIZE)),
        (
            "export-snippet",
            repeat_to(&unit("@@html:<b>x</b>@@"), SIZE),
        ),
        (
            "inline-src",
            repeat_to(&unit("src_python[:x y]{print(1)}"), SIZE),
        ),
        (
            "inline-babel-call",
            repeat_to(&unit("call_f[:a](x)[:b]"), SIZE),
        ),
        ("target", repeat_to(&unit("<<target>>"), SIZE)),
        ("radio-target", repeat_to(&unit("<<<radio>>>"), SIZE)),
        ("statistics-cookie", repeat_to(&unit("[1/3]"), SIZE)),
        ("line-break", repeat_to("words then a break\\\\\n", SIZE)),
        (
            "citation",
            repeat_to(&unit("[cite:@key1 p. 3; @key2]"), SIZE),
        ),
        ("mixed", repeat_to(&common::markup(SIZE), SIZE)),
    ]
}

fn objects(c: &mut Criterion) {
    let mut group = c.benchmark_group("objects");
    for (name, text) in inputs() {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &text, |b, text| {
            b.iter(|| Parser::new_inline(black_box(text)).count())
        });
    }
    group.finish();
}

/// The same `mixed` input under the options that change object parsing.
fn options(c: &mut Criterion) {
    let text = common::markup(SIZE);
    let cases: Vec<(&str, Options)> = vec![
        ("default", Options::default()),
        (
            "sub-superscripts-braces",
            Options {
                sub_superscripts: SubSuperscripts::Braces,
                ..Options::default()
            },
        ),
        (
            "sub-superscripts-never",
            Options {
                sub_superscripts: SubSuperscripts::Never,
                ..Options::default()
            },
        ),
        (
            "link-abbreviations",
            Options {
                link_abbreviations: vec![
                    ("gh".into(), "https://github.com/".into()),
                    ("doi".into(), "https://doi.org/%s".into()),
                ],
                ..Options::default()
            },
        ),
        (
            "link-types",
            Options {
                link_types: vec!["media".into(), "lecture".into(), "lecture-pdf".into()],
                ..Options::default()
            },
        ),
    ];
    let mut group = c.benchmark_group("objects-options");
    group.throughput(Throughput::Bytes(text.len() as u64));
    for (name, opts) in cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), &opts, |b, opts| {
            b.iter(|| Parser::new_inline_ext(black_box(&text), opts.clone()).count())
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
    targets = objects, options
}
criterion_main!(benches);
