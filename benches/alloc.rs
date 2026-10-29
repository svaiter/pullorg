//! Allocation profile. Timing varies from machine to machine; allocation
//! counts do not, which makes them the one benchmark number that can be
//! compared across commits without a baseline run. The library's rule is
//! "borrow from the input wherever possible; allocate only for transformed
//! text", and this report shows how well it holds per corpus:
//!
//! ```text
//! corpus            bytes   events    allocs  alloc'd KiB  allocs/KiB  borrowed text
//! notes             65536    12345       678         123       10.3        98.7 %
//! ```
//!
//! - `allocs`: heap allocations made while building the parser and draining
//!   the stream (`Parser::new(..).count()`), excluding the input itself.
//! - `alloc'd KiB`: bytes requested by those allocations.
//! - `borrowed text`: share of `Text`, `Code` and `Verbatim` events whose
//!   string is `CowStr::Borrowed`.
//! - the `html` column: allocations of parse + `push_html` with the output
//!   `String` pre-sized, i.e. the renderer's own allocations plus growth.
//!
//! This target is a plain binary (`harness = false`, no Criterion) so that
//! it runs the same under `cargo bench --bench alloc` and `cargo test
//! --benches`. Arguments are ignored except a substring filter on the
//! corpus name, and `--json` which writes one JSON object per line.
//!
//! The counting allocator is the only `unsafe` in the repository; it is a
//! pass-through to `System` that bumps two atomics.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use pullorg::{html, CowStr, Event, Parser};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is forwarded unchanged to `System`; the counters are
// plain atomics and do not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size(), Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(new_size.saturating_sub(layout.size()), Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Run `f` and return (allocations, bytes) it made.
fn measure<T>(f: impl FnOnce() -> T) -> (usize, usize, T) {
    let a0 = ALLOCS.load(Relaxed);
    let b0 = BYTES.load(Relaxed);
    let out = f();
    (ALLOCS.load(Relaxed) - a0, BYTES.load(Relaxed) - b0, out)
}

struct Row {
    name: String,
    bytes: usize,
    events: usize,
    allocs: usize,
    alloc_bytes: usize,
    borrowed: usize,
    text_events: usize,
    html_allocs: usize,
    html_bytes: usize,
}

fn profile(name: &str, text: &str) -> Row {
    let mut events = 0;
    let mut borrowed = 0;
    let mut text_events = 0;
    let (allocs, alloc_bytes, ()) = measure(|| {
        for ev in Parser::new(text) {
            events += 1;
            if let Event::Text(s) | Event::Code(s) | Event::Verbatim(s) = &ev {
                text_events += 1;
                if matches!(s, CowStr::Borrowed(_)) {
                    borrowed += 1;
                }
            }
        }
    });
    let mut out = String::with_capacity(text.len() * 2);
    let (html_allocs, html_bytes, ()) = measure(|| html::push_html(&mut out, Parser::new(text)));
    Row {
        name: name.to_string(),
        bytes: text.len(),
        events,
        allocs,
        alloc_bytes,
        borrowed,
        text_events,
        html_allocs,
        html_bytes,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let filter: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();
    let wanted = |name: &str| filter.is_empty() || filter.iter().any(|f| name.contains(f));

    let mut inputs: Vec<(String, String)> = Vec::new();
    for d in common::documents(common::DEFAULT_SIZE) {
        inputs.push((d.name.to_string(), d.text));
    }
    for d in common::adversarial(common::DEFAULT_SIZE) {
        inputs.push((format!("adversarial/{}", d.name), d.text));
    }
    inputs.extend(common::external());

    if !json {
        println!(
            "{:<32} {:>7} {:>8} {:>8} {:>11} {:>10} {:>13} {:>11} {:>11}",
            "corpus",
            "bytes",
            "events",
            "allocs",
            "alloc'd KiB",
            "allocs/KiB",
            "borrowed text",
            "html allocs",
            "html KiB"
        );
    }
    for (name, text) in inputs.iter().filter(|(n, _)| wanted(n)) {
        let r = profile(name, text);
        let borrowed_pct = if r.text_events == 0 {
            100.0
        } else {
            100.0 * r.borrowed as f64 / r.text_events as f64
        };
        if json {
            println!(
                "{{\"corpus\":\"{}\",\"bytes\":{},\"events\":{},\"allocs\":{},\"alloc_bytes\":{},\"borrowed_text\":{},\"text_events\":{},\"html_allocs\":{},\"html_alloc_bytes\":{}}}",
                r.name,
                r.bytes,
                r.events,
                r.allocs,
                r.alloc_bytes,
                r.borrowed,
                r.text_events,
                r.html_allocs,
                r.html_bytes
            );
        } else {
            println!(
                "{:<32} {:>7} {:>8} {:>8} {:>11.1} {:>10.2} {:>12.1} % {:>11} {:>11.1}",
                r.name,
                r.bytes,
                r.events,
                r.allocs,
                r.alloc_bytes as f64 / 1024.0,
                r.allocs as f64 / (r.bytes as f64 / 1024.0),
                borrowed_pct,
                r.html_allocs,
                r.html_bytes as f64 / 1024.0
            );
        }
    }
}
