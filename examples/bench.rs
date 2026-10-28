//! Rough in-process timing: `cargo run --release --example bench -- FILE...`
use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).unwrap();
        let n = 20;
        let t = Instant::now();
        let mut events = 0;
        for _ in 0..n {
            events = pullorg::Parser::new(&text).count();
        }
        let parse = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let t = Instant::now();
        for _ in 0..n {
            let mut out = String::new();
            pullorg::html::push_html(&mut out, pullorg::Parser::new(&text));
        }
        let render = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let name = std::path::Path::new(&path)
            .file_name()
            .unwrap()
            .to_string_lossy();
        println!(
            "{name:22} {:>7} KB {events:>7} events  parse: {parse:6.2} ms  parse+html: {render:6.2} ms",
            text.len() / 1024
        );
    }
}
