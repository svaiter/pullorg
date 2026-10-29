//! Scaling: the same corpus at 4, 16, 64 and 256 KiB. The parser is meant
//! to be linear in the input, so throughput (bytes/s, printed by Criterion)
//! should be about the same at every size. A line whose throughput falls
//! as the size grows is a super-linear path.
//!
//! `scaling/<document>/<size>` covers the realistic documents and
//! `scaling-adversarial/<case>/<size>` the inputs built to defeat caches and
//! bounded look-ahead. Running only the largest size of the adversarial
//! family is a quick smoke test:
//!
//! ```sh
//! cargo bench --bench scaling -- 'scaling-adversarial/.*/262144'
//! ```

mod common;

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use pullorg::Parser;

const SIZES: [usize; 4] = [4 * 1024, 16 * 1024, 64 * 1024, 256 * 1024];

fn documents(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling");
    for size in SIZES {
        for d in common::documents(size) {
            if d.name == "fixtures" {
                continue; // already measured in `parse`, does not scale interestingly
            }
            group.throughput(Throughput::Bytes(d.text.len() as u64));
            group.bench_with_input(BenchmarkId::new(d.name, size), &d.text, |b, text| {
                b.iter(|| Parser::new(black_box(text)).count())
            });
        }
    }
    group.finish();
}

fn adversarial(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling-adversarial");
    for size in SIZES {
        for d in common::adversarial(size) {
            group.throughput(Throughput::Bytes(d.text.len() as u64));
            group.bench_with_input(BenchmarkId::new(d.name, size), &d.text, |b, text| {
                b.iter(|| Parser::new(black_box(text)).count())
            });
        }
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
        .sample_size(20)
}

criterion_group! {
    name = benches;
    config = config();
    targets = documents, adversarial
}
criterion_main!(benches);
