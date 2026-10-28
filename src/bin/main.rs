use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser as ClapParser;
use pullorg::{html, Event, Options, Parser, SubSuperscripts};

/// Convert Org-mode documents to HTML.
#[derive(ClapParser, Debug)]
#[command(name = "pullorg", version, about)]
struct Cli {
    /// Input file; reads standard input when absent or `-`.
    file: Option<PathBuf>,

    /// Write the output to this file instead of standard output.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Print the event stream with byte ranges instead of HTML.
    #[arg(long)]
    events: bool,

    /// Wrap the output in a minimal HTML5 document.
    #[arg(long)]
    standalone: bool,

    /// With --standalone, load MathJax from a CDN to typeset LaTeX fragments.
    #[arg(long)]
    mathjax: bool,

    /// Document title for --standalone (default: `#+TITLE:` or the file name).
    #[arg(long)]
    title: Option<String>,

    /// Do not interpret `_` and `^` as subscript and superscript.
    #[arg(long)]
    no_sub_superscript: bool,

    /// Comma separated TODO keywords (open tasks).
    #[arg(long, value_name = "KEYWORDS")]
    todo: Option<String>,

    /// Comma separated DONE keywords (closed tasks).
    #[arg(long, value_name = "KEYWORDS")]
    done: Option<String>,
}

const STYLE: &str = "\
body { max-width: 50em; margin: 2em auto; padding: 0 1em; line-height: 1.5; font-family: sans-serif; }
pre { background: #f5f5f5; border: 1px solid #ddd; padding: 0.6em; overflow-x: auto; }
code { font-family: monospace; background: #f5f5f5; padding: 0 0.2em; }
pre code { background: none; padding: 0; }
.todo { color: #c00; font-weight: bold; }
.done { color: #080; font-weight: bold; }
.tag { float: right; font-size: 80%; font-weight: normal; }
.tag span { background: #eee; border-radius: 3px; padding: 0 0.3em; }
.timestamp { color: #666; font-family: monospace; }
table { border-collapse: collapse; margin: 1em 0; }
th, td { border: 1px solid #ccc; padding: 0.2em 0.6em; }
th { background: #f0f0f0; }
.footdef { margin: 0.4em 0; }
.footpara { display: inline; }
.footpara > p:first-child { display: inline; }
#footnotes { border-top: 1px solid #ccc; margin-top: 2em; font-size: 90%; }
blockquote { border-left: 3px solid #ccc; margin-left: 0; padding-left: 1em; color: #444; }
";

const MATHJAX: &str =
    "<script async src=\"https://cdn.jsdelivr.net/npm/mathjax@3/es5/tex-mml-chtml.js\"></script>\n";

fn split_keywords(s: &str) -> Vec<String> {
    s.split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn run(cli: Cli) -> Result<(), String> {
    let (source, name) = match &cli.file {
        Some(p) if p.as_os_str() != "-" => {
            let text =
                fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (text, name)
        }
        _ => {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| format!("cannot read standard input: {e}"))?;
            (text, "stdin".to_string())
        }
    };

    let mut options = Options::default();
    if cli.no_sub_superscript {
        options.sub_superscripts = SubSuperscripts::Never;
    }
    if let Some(t) = &cli.todo {
        options.todo_keywords = split_keywords(t);
    }
    if let Some(d) = &cli.done {
        options.done_keywords = split_keywords(d);
    }

    let mut out = String::new();
    if cli.events {
        use std::fmt::Write as _;
        for (event, range) in Parser::new_ext(&source, options).into_offset_iter() {
            let _ = writeln!(out, "{}..{} {:?}", range.start, range.end, event);
        }
    } else {
        let mut title = cli.title.clone();
        let mut events = Vec::new();
        for event in Parser::new_ext(&source, options) {
            if title.is_none() {
                if let Event::Keyword(k) = &event {
                    if k.key.eq_ignore_ascii_case("TITLE") {
                        title = Some(k.value.to_string());
                    }
                }
            }
            events.push(event);
        }
        let mut body = String::new();
        html::push_html(&mut body, events.into_iter());
        if cli.standalone {
            let title = title.unwrap_or(name);
            out.push_str(&format!(
                "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>\n{STYLE}</style>\n{}</head>\n<body>\n",
                escape(&title),
                if cli.mathjax { MATHJAX } else { "" }
            ));
            out.push_str(&body);
            out.push_str("</body>\n</html>\n");
        } else {
            out = body;
        }
    }

    match &cli.output {
        Some(p) => fs::write(p, out).map_err(|e| format!("cannot write {}: {e}", p.display())),
        None => {
            let mut stdout = io::stdout().lock();
            stdout
                .write_all(out.as_bytes())
                .and_then(|_| stdout.flush())
                .map_err(|e| format!("cannot write to standard output: {e}"))
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("pullorg: {msg}");
            ExitCode::FAILURE
        }
    }
}
