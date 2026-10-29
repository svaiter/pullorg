//! HTML rendering.
//!
//! `html/<corpus>` is what the CLI does: parse and render in one pass into a
//! fresh `String`. `html-render-only/<corpus>` renders an already collected
//! event stream, isolating the renderer from the parser (the clone of each
//! borrowed event is cheap). `html-write-io/notes` goes through
//! `write_html_io` into a sink to measure the `io::Write` path.

mod common;

use std::hint::black_box;
use std::io;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use pullorg::{html, Event, Parser};

fn html_full(c: &mut Criterion) {
    let docs = common::documents(common::DEFAULT_SIZE);
    let ext = common::external();
    let inputs = docs
        .iter()
        .map(|d| (d.name.to_string(), d.text.as_str()))
        .chain(ext.iter().map(|(n, t)| (n.clone(), t.as_str())));

    let mut group = c.benchmark_group("html");
    for (name, text) in inputs {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&name), text, |b, text| {
            b.iter(|| {
                let mut out = String::new();
                html::push_html(&mut out, Parser::new(black_box(text)));
                out
            })
        });
    }
    group.finish();
}

fn html_render_only(c: &mut Criterion) {
    let docs = common::documents(common::DEFAULT_SIZE);
    let mut group = c.benchmark_group("html-render-only");
    for d in &docs {
        let events: Vec<Event> = Parser::new(&d.text).collect();
        group.throughput(Throughput::Bytes(d.text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(d.name), &events, |b, events| {
            b.iter_batched(
                || String::with_capacity(d.text.len() * 2),
                |mut out| {
                    html::push_html(&mut out, events.iter().cloned());
                    out
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn html_write_io(c: &mut Criterion) {
    let text = common::notes(common::DEFAULT_SIZE);
    let mut group = c.benchmark_group("html-write-io");
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("notes", |b| {
        b.iter(|| {
            let mut sink = io::sink();
            html::write_html_io(&mut sink, Parser::new(black_box(&text))).unwrap();
        })
    });
    group.bench_function("notes-vec", |b| {
        b.iter(|| {
            let mut buf = Vec::new();
            html::write_html_io(&mut buf, Parser::new(black_box(&text))).unwrap();
            buf
        })
    });
    group.finish();
}

/// Shorter than Criterion's defaults; the inputs are small and the suite is
/// large. Command-line flags (`--warm-up-time`, `--measurement-time`,
/// `--quick`) still override these.
fn config() -> Criterion {
    Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3))
}

criterion_group! {
    name = benches;
    config = config();
    targets = html_full, html_render_only, html_write_io
}
criterion_main!(benches);
