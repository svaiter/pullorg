//! End-to-end parsing throughput: build a `Parser` and drain its stream.
//!
//! `parse/<corpus>` consumes every event; `parse-offsets/<corpus>` does the
//! same through `into_offset_iter`; `parse-collect/<corpus>` collects the
//! events into a `Vec`, the way a renderer that needs look-ahead would.
//! Throughput is reported in bytes of input per second, so corpora of
//! different sizes compare directly.

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use pullorg::{Event, Parser};

fn count(text: &str) -> usize {
    Parser::new(black_box(text)).count()
}

fn parse(c: &mut Criterion) {
    let docs = common::documents(common::DEFAULT_SIZE);
    let ext = common::external();
    let inputs = docs
        .iter()
        .map(|d| (d.name.to_string(), d.text.as_str()))
        .chain(ext.iter().map(|(n, t)| (n.clone(), t.as_str())));

    let mut group = c.benchmark_group("parse");
    for (name, text) in inputs {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&name), text, |b, text| {
            b.iter(|| count(text))
        });
    }
    group.finish();
}

fn parse_offsets(c: &mut Criterion) {
    let docs = common::documents(common::DEFAULT_SIZE);
    let mut group = c.benchmark_group("parse-offsets");
    for d in docs.iter().filter(|d| matches!(d.name, "notes" | "markup")) {
        group.throughput(Throughput::Bytes(d.text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(d.name), &d.text, |b, text| {
            b.iter(|| {
                Parser::new(black_box(text))
                    .into_offset_iter()
                    .fold(0usize, |acc, (_, r)| acc.wrapping_add(r.end))
            })
        });
    }
    group.finish();
}

fn parse_collect(c: &mut Criterion) {
    let docs = common::documents(common::DEFAULT_SIZE);
    let mut group = c.benchmark_group("parse-collect");
    for d in docs.iter().filter(|d| matches!(d.name, "notes" | "markup")) {
        group.throughput(Throughput::Bytes(d.text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(d.name), &d.text, |b, text| {
            b.iter(|| Parser::new(black_box(text)).collect::<Vec<Event>>())
        });
    }
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
    targets = parse, parse_offsets, parse_collect
}
criterion_main!(benches);
