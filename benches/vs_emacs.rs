//! pullorg side of the comparison with Emacs' `org-element-parse-buffer`.
//!
//! Driven by `tools/bench-vs-emacs.sh`, which runs this target, then
//! `tools/bench-emacs.el` on the same files, then joins the two reports.
//! Nothing it reads or writes is checked in: the corpora are regenerated on
//! every run from `common` (plus the files of `PULLORG_BENCH_CORPUS`, when
//! set) and written to `--out DIR/corpus`, so Emacs parses exactly the
//! bytes pullorg parsed.
//!
//! ```text
//! cargo bench --bench vs_emacs -- --out DIR [--samples N] [--warmup N] [FILTER]
//! ```
//!
//! Corpora: every document corpus at 4, 16, 64 and 256 KiB
//! (`notes-64k`, ...), then the external files under their own name.
//! `FILTER` keeps the corpora whose name contains it.
//!
//! Two levels are timed, matching two granularities of
//! `org-element-parse-buffer`:
//!
//! - `elements`: `Parser::new(text)`, which builds the element tree and
//!   leaves inline objects unparsed. Compared with granularity `element`.
//!   Approximate: pullorg also discovers radio targets at this point.
//! - `full`: `Parser::new(text).count()`, every event including inline
//!   objects. Compared with granularity `object`, the default.
//!
//! The measurement rule is the same on both sides (keep them in sync with
//! `tools/bench-emacs.el`): one calibration run picks the iteration count
//! that makes a sample last at least `TARGET_SAMPLE`; `warmup` samples are
//! discarded; `samples` samples are timed; the report gives the median,
//! minimum and maximum time per iteration. Results go to `DIR/pullorg.tsv`,
//! one line per corpus and level:
//!
//! ```text
//! name  bytes  level  median_s  min_s  max_s  gc_s
//! ```
//!
//! (`gc_s`, Emacs' mean GC time per iteration, is always 0 here.)
//!
//! Under `cargo test --benches` (no `--bench` argument) every corpus is
//! generated small and parsed once, nothing is written. Under a plain
//! `cargo bench` without `--out` it prints how to run it and exits.

mod common;

use std::fmt::Write as _;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pullorg::Parser;

const SIZES: &[(usize, &str)] = &[
    (4 * 1024, "4k"),
    (16 * 1024, "16k"),
    (64 * 1024, "64k"),
    (256 * 1024, "256k"),
];

const TARGET_SAMPLE: Duration = Duration::from_millis(10);

struct Args {
    bench: bool,
    out: Option<PathBuf>,
    samples: usize,
    warmup: usize,
    filter: Option<String>,
}

fn args() -> Args {
    let mut a = Args {
        bench: false,
        out: None,
        samples: 20,
        warmup: 3,
        filter: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--bench" => a.bench = true,
            "--out" => a.out = Some(it.next().expect("--out DIR").into()),
            "--samples" => a.samples = num(it.next(), "--samples"),
            "--warmup" => a.warmup = num(it.next(), "--warmup"),
            s if s.starts_with('-') => {} // flags cargo or libtest may pass
            s => a.filter = Some(s.to_string()),
        }
    }
    a
}

fn num(v: Option<String>, flag: &str) -> usize {
    v.and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("{flag} needs a number"))
}

fn corpora(sizes: &[(usize, &str)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for &(size, label) in sizes {
        for c in common::documents(size) {
            out.push((format!("{}-{label}", c.name), c.text));
        }
    }
    out.extend(common::external());
    out
}

/// Seconds per iteration of each timed sample, after calibration and warm-up.
fn measure(samples: usize, warmup: usize, mut f: impl FnMut()) -> Vec<f64> {
    let t = Instant::now();
    f();
    let once = t.elapsed().max(Duration::from_nanos(1));
    let iters = (TARGET_SAMPLE.as_secs_f64() / once.as_secs_f64())
        .ceil()
        .max(1.0) as usize;
    let mut times = Vec::with_capacity(samples);
    for i in 0..warmup + samples {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        if i >= warmup {
            times.push(t.elapsed().as_secs_f64() / iters as f64);
        }
    }
    times
}

fn summary(mut times: Vec<f64>) -> (f64, f64, f64) {
    times.sort_by(f64::total_cmp);
    let n = times.len();
    let median = if n % 2 == 1 {
        times[n / 2]
    } else {
        (times[n / 2 - 1] + times[n / 2]) / 2.0
    };
    (median, times[0], times[n - 1])
}

fn main() {
    let a = args();
    if !a.bench {
        // `cargo test --benches`: check every corpus parses, quickly.
        for (_, text) in corpora(&[(2 * 1024, "2k")]) {
            black_box(Parser::new(&text).count());
        }
        return;
    }
    let Some(out) = a.out else {
        eprintln!("vs_emacs: skipped; run `sh tools/bench-vs-emacs.sh` to compare with Emacs");
        return;
    };
    assert!(a.samples > 0, "--samples must be positive");

    let dir = out.join("corpus");
    std::fs::create_dir_all(&dir).expect("create corpus directory");
    let mut report = String::new();
    for (name, text) in corpora(SIZES) {
        if a.filter.as_deref().is_some_and(|f| !name.contains(f)) {
            continue;
        }
        std::fs::write(dir.join(format!("{name}.org")), &text).expect("write corpus");
        let levels: [(&str, &dyn Fn()); 2] = [
            ("elements", &|| {
                drop(black_box(Parser::new(black_box(&text))))
            }),
            ("full", &|| {
                black_box(Parser::new(black_box(&text)).count());
            }),
        ];
        for (level, f) in levels {
            let (median, min, max) = summary(measure(a.samples, a.warmup, f));
            eprintln!("pullorg {name:<20} {level:<8} {:>10.3} ms", median * 1e3);
            writeln!(
                report,
                "{name}\t{}\t{level}\t{median:.9}\t{min:.9}\t{max:.9}\t0",
                text.len()
            )
            .unwrap();
        }
    }
    std::fs::write(out.join("pullorg.tsv"), report).expect("write pullorg.tsv");
}
