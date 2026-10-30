//! HTML renderer.
//!
//! Mirrors `pulldown_cmark::html`: feed any `Iterator<Item = Event>` to
//! [`push_html`], [`write_html_fmt`] or [`write_html_io`].
//!
//! The output is an HTML fragment (no `<html>` or `<body>`), loosely modelled
//! on Emacs' `ox-html` but flatter: headlines become `<h2>`..`<h6>` (`<h1>` is
//! left to the document title) and no section wrappers are produced.
//!
//! The renderer streams into the sink and only buffers where look-ahead is
//! required: tables, footnote definitions, radio targets and paragraphs
//! carrying affiliated keywords. Headlines are held back until their property
//! drawer has been seen, so that a `CUSTOM_ID` property can become the `id`.
//!
//! # Headline ids and links
//!
//! Every rendered headline gets `id="outline-N"` (`N` counting rendered
//! headlines from 1 in document order), or its `CUSTOM_ID` when it has one.
//! `[[#id]]` links point at `#id`. Fuzzy links such as `[[*Title]]` are *not*
//! resolved to headlines: that would need a pre-scan of the whole document,
//! which the streaming renderer does not do, so they keep pointing at `#path`.
//!
//! # Macros
//!
//! `#+MACRO: name template` defines a macro; `{{{name(a,b)}}}` expands the
//! template, replacing `$1`, `$2`, ... by the arguments (missing ones become
//! empty). The built-ins of `org-macro` are supported (`title`, `author`,
//! `email`, `date`, `language`, `subtitle`, `keyword(KEY)`, `n`, `n(NAME)`,
//! `kbd(x)`; `results`, `property`, `time`, `modification-time` and
//! `input-file` expand to nothing). Unlike Emacs, an expansion is inserted as
//! escaped plain text and is not parsed as Org again, and a macro must be
//! defined before its first use. Unknown macros expand to nothing.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::io;

#[cfg(test)]
use crate::event::Priority;
use crate::event::{
    Checkbox, Event, Headline, Keyword, LinkFormat, ListKind, TableKind, TableRowKind, Tag, TagEnd,
    TodoType,
};

/// Render events as HTML, appending to `output`.
pub fn push_html<'a, I>(output: &mut String, iter: I)
where
    I: Iterator<Item = Event<'a>>,
{
    let _ = write_html_fmt(output, iter);
}

/// Render events as HTML into a [`fmt::Write`].
pub fn write_html_fmt<'a, I, W>(writer: W, iter: I) -> fmt::Result
where
    I: Iterator<Item = Event<'a>>,
    W: fmt::Write,
{
    let mut r = Renderer::new(writer);
    r.feed(iter)?;
    r.finish()
}

/// Render events as HTML into an [`io::Write`].
pub fn write_html_io<'a, I, W>(writer: W, iter: I) -> io::Result<()>
where
    I: Iterator<Item = Event<'a>>,
    W: io::Write,
{
    struct Adapter<W: io::Write> {
        inner: W,
        error: Option<io::Error>,
    }
    impl<W: io::Write> fmt::Write for Adapter<W> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            match self.inner.write_all(s.as_bytes()) {
                Ok(()) => Ok(()),
                Err(e) => {
                    self.error = Some(e);
                    Err(fmt::Error)
                }
            }
        }
    }
    let mut adapter = Adapter {
        inner: writer,
        error: None,
    };
    match write_html_fmt(&mut adapter, iter) {
        Ok(()) => Ok(()),
        Err(_) => Err(adapter
            .error
            .unwrap_or_else(|| io::Error::other("formatter error"))),
    }
}

/// Escape `&`, `<`, `>` and `"`.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Whether block `switches` ask to preserve indentation: `-i` as a whole
/// word, like org-element's `(string-match-p "-i\\>" switches)`. Word
/// constituents follow the Org buffer syntax table: letters, digits, `@`
/// and `'`.
fn preserves_indent(switches: Option<&str>) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '@' || c == '\'';
    switches.is_some_and(|s| {
        s.match_indices("-i")
            .any(|(i, _)| !s[i + 2..].chars().next().is_some_and(is_word))
    })
}

/// Leading indentation of `line` (spaces and tabs) as `(column, bytes)`,
/// with tab stops every 8 columns.
fn indentation(line: &str) -> (usize, usize) {
    let mut col = 0;
    let mut len = 0;
    for b in line.bytes() {
        match b {
            b' ' => col += 1,
            b'\t' => col = (col / 8 + 1) * 8,
            _ => break,
        }
        len += 1;
    }
    (col, len)
}

/// Remove the common indentation of `s`, like Emacs' `org-remove-indentation`
/// (used by ox-html on src and example block bodies).
///
/// The common indentation `n` is the minimum indentation column of the lines
/// holding a non-blank character. Nothing changes when it is 0. Otherwise a
/// line indented by less than `n` columns is emptied when it is blank (and
/// the whole removal is abandoned when it is not, which only happens for lines
/// starting with `\r` or `\f`), and the other lines lose the leading
/// whitespace characters lying within the first `n` columns. Like Emacs with
/// its default `indent-tabs-mode`, a tab straddling column `n` is kept, and
/// whatever follows column `n` is kept verbatim, so blank lines indented
/// deeper than `n` keep their extra whitespace. When no line holds a
/// non-blank character, Emacs takes the buffer size plus one as `n`.
fn remove_indentation(s: &str) -> Cow<'_, str> {
    let mut n = usize::MAX;
    for line in s.split('\n') {
        let (col, len) = indentation(line);
        // Emacs' regexp `^[ \t]*\S-`: `\r` and `\f` are whitespace too.
        if !matches!(line[len..].chars().next(), None | Some('\r' | '\x0c')) {
            if col == 0 {
                return Cow::Borrowed(s);
            }
            n = n.min(col);
        }
    }
    if n == usize::MAX {
        n = s.chars().count() + 1;
    }
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (col, len) = indentation(line);
        if col < n {
            if len == line.len() {
                continue;
            }
            return Cow::Borrowed(s);
        }
        let mut c = 0;
        let mut cut = 0;
        for b in line[..len].bytes() {
            let next = if b == b'\t' { (c / 8 + 1) * 8 } else { c + 1 };
            if next > n {
                break;
            }
            c = next;
            cut += 1;
            if c == n {
                break;
            }
        }
        out.push_str(&line[cut..]);
    }
    Cow::Owned(out)
}

/// Attributes derived from affiliated keywords.
#[derive(Default, Debug)]
struct Aff {
    id: Option<String>,
    class: Option<String>,
    attrs: Vec<(String, String)>,
    caption: Option<String>,
}

impl Aff {
    fn from_keywords(kws: &[(String, String)]) -> Aff {
        let mut a = Aff::default();
        for (k, v) in kws {
            match k.as_str() {
                "NAME" => a.id = Some(v.clone()),
                "CAPTION" => a.caption = Some(v.clone()),
                "ATTR_HTML" => a.parse_attr_html(v),
                _ => {}
            }
        }
        a
    }

    fn is_empty(&self) -> bool {
        self.id.is_none() && self.class.is_none() && self.attrs.is_empty() && self.caption.is_none()
    }

    fn parse_attr_html(&mut self, value: &str) {
        let mut key: Option<&str> = None;
        let mut vals: Vec<&str> = Vec::new();
        let mut toks: Vec<&str> = value.split_whitespace().collect();
        toks.push(":");
        for tok in toks {
            if let Some(rest) = tok.strip_prefix(':') {
                if let Some(k) = key {
                    self.add(k, &vals.join(" "));
                }
                key = if rest.is_empty() { None } else { Some(rest) };
                vals.clear();
            } else {
                vals.push(tok);
            }
        }
    }

    fn add(&mut self, key: &str, value: &str) {
        let value = value.trim_matches('"').to_string();
        match key {
            "id" => self.id = Some(value),
            "class" => match &mut self.class {
                Some(c) => {
                    c.push(' ');
                    c.push_str(&value)
                }
                None => self.class = Some(value),
            },
            _ => self.attrs.push((key.to_string(), value)),
        }
    }

    fn attr_string(&self, base: Option<&str>, with_id: bool) -> String {
        let mut s = String::new();
        if with_id {
            if let Some(id) = &self.id {
                s.push_str(&format!(" id=\"{}\"", escape(id)));
            }
        }
        let class = match (base, &self.class) {
            (Some(b), Some(c)) => Some(format!("{b} {c}")),
            (Some(b), None) => Some(b.to_string()),
            (None, Some(c)) => Some(c.clone()),
            (None, None) => None,
        };
        if let Some(c) = class {
            s.push_str(&format!(" class=\"{}\"", escape(&c)));
        }
        for (k, v) in &self.attrs {
            s.push_str(&format!(" {}=\"{}\"", escape(k), escape(v)));
        }
        s
    }

    fn open(&self, tag: &str, base: Option<&str>) -> String {
        format!("<{tag}{}>", self.attr_string(base, true))
    }
}

/// What the renderer needs to know about the event following the current one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    EndLink,
    StartItemTag,
    EndContainer,
    Other,
}

/// How the currently open paragraph is rendered.
enum ParaKind {
    Plain,
    /// First child of an item or footnote definition: no `<p>`.
    Lead,
    Aff(Aff),
}

/// A rendered headline line waiting for the property drawer.
struct Held {
    level: usize,
    id: usize,
    custom_id: Option<String>,
    inner: String,
}

struct HInfo {
    id: usize,
    level: usize,
    inline: bool,
    archived: bool,
    muted: bool,
    prefix: String,
    tags: Vec<String>,
}

struct ItemInfo {
    desc: bool,
    checkbox: Option<Checkbox>,
}

struct FnEntry {
    label: Option<String>,
    refs: usize,
    def: Option<String>,
}

struct Renderer<W: fmt::Write> {
    out: W,
    bufs: Vec<String>,
    skip: usize,
    raw: bool,
    verse: usize,
    pending: Vec<(String, String)>,
    hstack: Vec<HInfo>,
    lists: Vec<ListKind>,
    items: Vec<ItemInfo>,
    paras: Vec<ParaKind>,
    lead: bool,
    held: Option<Held>,
    headline_count: usize,
    macros: HashMap<String, String>,
    kw_values: HashMap<String, String>,
    counters: HashMap<String, i64>,
    src_caption: Vec<bool>,
    preserve_indent: Vec<bool>,
    export_html: Vec<bool>,
    entries: Vec<FnEntry>,
    label_idx: HashMap<String, usize>,
    pending_defs: HashMap<String, String>,
    fn_def_stack: Vec<String>,
    fn_inline_stack: Vec<usize>,
    radio_text: Vec<String>,
    cite_first: bool,
}

/// Rewrite dollar-delimited LaTeX fragments to the `\(...\)` and `\[...\]`
/// forms MathJax recognises by default (single dollars are off there), as
/// Emacs' HTML export does: `$x$` becomes `\(x\)`, `$$x$$` becomes `\[x\]`.
fn mathjax_delimiters(s: &str) -> Cow<'_, str> {
    if let Some(inner) = s.strip_prefix("$$").and_then(|r| r.strip_suffix("$$")) {
        Cow::Owned(format!("\\[{inner}\\]"))
    } else if let Some(inner) = s.strip_prefix('$').and_then(|r| r.strip_suffix('$')) {
        Cow::Owned(format!("\\({inner}\\)"))
    } else {
        Cow::Borrowed(s)
    }
}

fn is_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ["png", "jpg", "jpeg", "gif", "svg", "webp"]
        .iter()
        .any(|e| lower.rsplit_once('.').is_some_and(|(_, x)| x == *e))
}

/// Replace `$N` (N >= 1) in a macro template by the N-th argument.
fn expand_template(tpl: &str, args: &[crate::strings::CowStr<'_>]) -> String {
    let mut out = String::new();
    let mut rest = tpl;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        match after[..digits].parse::<usize>() {
            Ok(n) if n >= 1 => {
                if let Some(a) = args.get(n - 1) {
                    out.push_str(a);
                }
                rest = &after[digits..];
            }
            _ => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn checkbox_html(c: Checkbox) -> &'static str {
    match c {
        Checkbox::On => "<code>[X]</code> ",
        Checkbox::Off => "<code>[ ]</code> ",
        Checkbox::Trans => "<code>[-]</code> ",
    }
}

impl<W: fmt::Write> Renderer<W> {
    fn new(out: W) -> Self {
        Renderer {
            out,
            bufs: Vec::new(),
            skip: 0,
            raw: false,
            verse: 0,
            pending: Vec::new(),
            hstack: Vec::new(),
            lists: Vec::new(),
            items: Vec::new(),
            paras: Vec::new(),
            lead: false,
            held: None,
            headline_count: 0,
            macros: HashMap::new(),
            kw_values: HashMap::new(),
            counters: HashMap::new(),
            src_caption: Vec::new(),
            preserve_indent: Vec::new(),
            export_html: Vec::new(),
            entries: Vec::new(),
            label_idx: HashMap::new(),
            pending_defs: HashMap::new(),
            fn_def_stack: Vec::new(),
            fn_inline_stack: Vec::new(),
            radio_text: Vec::new(),
            cite_first: true,
        }
    }

    fn w(&mut self, s: &str) -> fmt::Result {
        match self.bufs.last_mut() {
            Some(b) => {
                b.push_str(s);
                Ok(())
            }
            None => self.out.write_str(s),
        }
    }

    fn we(&mut self, s: &str) -> fmt::Result {
        self.w(&escape(s))
    }

    fn pop_buf(&mut self) -> String {
        self.bufs.pop().unwrap_or_default()
    }

    fn feed<'e, I: Iterator<Item = Event<'e>>>(&mut self, iter: I) -> fmt::Result {
        let mut it = iter.peekable();
        while let Some(ev) = it.next() {
            if self.skip == 0 {
                if let Event::Start(Tag::Table { kind, .. }) = &ev {
                    let kind = *kind;
                    self.flush_held()?;
                    let kws = std::mem::take(&mut self.pending);
                    let mut depth = 1usize;
                    let mut evs = Vec::new();
                    for e in it.by_ref() {
                        match &e {
                            Event::Start(_) => depth += 1,
                            Event::End(_) => depth -= 1,
                            _ => {}
                        }
                        if depth == 0 {
                            break;
                        }
                        evs.push(e);
                    }
                    self.table(kind, evs, &kws)?;
                    continue;
                }
            }
            let next = match it.peek() {
                Some(Event::End(TagEnd::Link)) => Next::EndLink,
                Some(Event::Start(Tag::ItemTag)) => Next::StartItemTag,
                Some(Event::End(TagEnd::Item | TagEnd::FootnoteDefinition)) => Next::EndContainer,
                _ => Next::Other,
            };
            if self.handle(ev, next)? {
                it.next();
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> fmt::Result {
        self.flush_held()?;
        if self.entries.is_empty() {
            return Ok(());
        }
        self.w("<div id=\"footnotes\"><h2 class=\"footnotes\">Footnotes</h2><div id=\"text-footnotes\">\n")?;
        let mut i = 0;
        while i < self.entries.len() {
            let n = i + 1;
            let def = match self.entries[i].def.clone() {
                Some(d) => d,
                None => self.entries[i]
                    .label
                    .as_ref()
                    .and_then(|l| self.pending_defs.get(l).cloned())
                    .unwrap_or_default(),
            };
            self.w(&format!(
                "<div class=\"footdef\"><sup><a id=\"fn.{n}\" class=\"footnum\" href=\"#fnr.{n}\">{n}</a></sup> <div class=\"footpara\">{def}</div></div>\n"
            ))?;
            i += 1;
        }
        self.w("</div></div>\n")
    }

    fn footnote_ref(&mut self, label: Option<&str>) -> (usize, String) {
        let idx = match label.and_then(|l| self.label_idx.get(l).copied()) {
            Some(i) => i,
            None => {
                self.entries.push(FnEntry {
                    label: label.map(str::to_string),
                    refs: 0,
                    def: None,
                });
                let i = self.entries.len() - 1;
                if let Some(l) = label {
                    self.label_idx.insert(l.to_string(), i);
                }
                i
            }
        };
        self.entries[idx].refs += 1;
        let r = self.entries[idx].refs;
        let n = idx + 1;
        let id = if r == 1 {
            format!("fnr.{n}")
        } else {
            format!("fnr.{n}.{}", 100 + r - 2)
        };
        (n, id)
    }

    /// Returns `true` when the following event has been consumed.
    fn handle(&mut self, ev: Event<'_>, next: Next) -> Result<bool, fmt::Error> {
        if self.skip > 0 {
            match ev {
                Event::Start(_) => self.skip += 1,
                Event::End(_) => self.skip -= 1,
                _ => {}
            }
            return Ok(false);
        }
        if self.held.is_some() {
            match &ev {
                Event::NodeProperty { key, value } => {
                    if key.eq_ignore_ascii_case("CUSTOM_ID") {
                        if let Some(h) = &mut self.held {
                            h.custom_id = Some(value.trim().to_string());
                        }
                    }
                }
                Event::Start(Tag::Section | Tag::PropertyDrawer)
                | Event::End(TagEnd::PropertyDrawer)
                | Event::Planning { .. } => {}
                _ => self.flush_held()?,
            }
        }
        let lead = std::mem::take(&mut self.lead);
        let kws = match &ev {
            Event::Keyword(Keyword {
                affiliated: true, ..
            }) => Vec::new(),
            _ => std::mem::take(&mut self.pending),
        };
        match ev {
            Event::Start(tag) => return self.start(tag, &kws, next, lead),
            Event::End(end) => self.end(end, next)?,
            Event::Text(t) => {
                if let Some(r) = self.radio_text.last_mut() {
                    r.push_str(&t);
                }
                if self.raw {
                    self.w(&t)?;
                } else {
                    self.we(&t)?;
                }
            }
            Event::Code(t) | Event::Verbatim(t) => {
                self.w("<code>")?;
                self.we(&t)?;
                self.w("</code>")?;
            }
            Event::Entity(e) => self.w(e.html)?,
            Event::LatexFragment(t) => self.we(&mathjax_delimiters(&t))?,
            Event::LatexEnvironment(t) => {
                self.we(&t)?;
                self.w("\n")?;
            }
            Event::ExportSnippet { backend, value } => {
                if backend.eq_ignore_ascii_case("html") {
                    self.w(&value)?;
                }
            }
            Event::InlineSrcBlock {
                language, value, ..
            } => {
                self.w(&format!("<code class=\"src src-{}\">", escape(&language)))?;
                self.we(&value)?;
                self.w("</code>")?;
            }
            Event::InlineBabelCall { .. } => {}
            Event::Macro { key, args } => self.expand_macro(&key, &args)?,
            Event::Target(name) => self.w(&format!("<a id=\"{}\"></a>", escape(&name)))?,
            Event::StatisticsCookie(c) => {
                self.w("<code>")?;
                self.we(&c)?;
                self.w("</code>")?;
            }
            Event::Timestamp(ts) => {
                self.w("<span class=\"timestamp-wrapper\"><span class=\"timestamp\">")?;
                self.we(&ts.raw)?;
                self.w("</span></span>")?;
            }
            Event::FootnoteReference(label) => {
                let (n, id) = self.footnote_ref(Some(&label));
                self.fn_link(n, &id)?;
            }
            Event::CitationReference {
                key,
                prefix,
                suffix,
            } => {
                if !self.cite_first {
                    self.w("; ")?;
                }
                self.cite_first = false;
                if let Some(p) = prefix {
                    self.we(p.trim())?;
                    self.w(" ")?;
                }
                self.w("@")?;
                self.we(&key)?;
                if let Some(s) = suffix {
                    self.w(" ")?;
                    self.we(s.trim())?;
                }
            }
            Event::LineBreak => self.w("<br />\n")?,
            Event::SoftBreak => {
                if self.verse > 0 {
                    self.w("<br />\n")?
                } else {
                    self.w("\n")?
                }
            }
            Event::Rule => self.w("<hr />\n")?,
            Event::Keyword(k) => {
                if k.affiliated {
                    self.pending.push((k.key.to_string(), k.value.to_string()));
                } else if k.key.eq_ignore_ascii_case("MACRO") {
                    let v = k.value.trim();
                    let (name, tpl) = match v.split_once(char::is_whitespace) {
                        Some((n, t)) => (n, t.trim_start()),
                        None => (v, ""),
                    };
                    if !name.is_empty() {
                        self.macros
                            .insert(name.to_ascii_lowercase(), tpl.to_string());
                    }
                } else {
                    self.kw_values
                        .insert(k.key.to_ascii_uppercase(), k.value.to_string());
                }
            }
            Event::FixedWidth(t) => {
                let aff = Aff::from_keywords(&kws);
                self.w(&aff.open("pre", Some("example")))?;
                self.we(t.strip_suffix('\n').unwrap_or(&t))?;
                self.w("</pre>\n")?;
            }
            Event::Comment(_)
            | Event::BabelCall { .. }
            | Event::DiarySexp(_)
            | Event::Clock { .. }
            | Event::Planning { .. }
            | Event::NodeProperty { .. } => {}
        }
        Ok(false)
    }

    fn flush_held(&mut self) -> fmt::Result {
        if let Some(h) = self.held.take() {
            let id = h.custom_id.unwrap_or_else(|| format!("outline-{}", h.id));
            let n = (h.level + 1).min(6);
            self.w(&format!(
                "<h{n} id=\"{}\">{}</h{n}>\n",
                escape(&id),
                h.inner
            ))?;
        }
        Ok(())
    }

    fn expand_macro(&mut self, key: &str, args: &[crate::strings::CowStr<'_>]) -> fmt::Result {
        if let Some(tpl) = self.macros.get(key) {
            let out = expand_template(tpl, args);
            return self.we(&out);
        }
        let arg = |i: usize| args.get(i).map(|a| a.trim()).unwrap_or("");
        match key {
            "title" | "author" | "email" | "date" | "language" | "subtitle" => {
                let v = self
                    .kw_values
                    .get(&key.to_ascii_uppercase())
                    .cloned()
                    .unwrap_or_default();
                self.we(&v)
            }
            "keyword" => {
                let v = self
                    .kw_values
                    .get(&arg(0).to_ascii_uppercase())
                    .cloned()
                    .unwrap_or_default();
                self.we(&v)
            }
            "n" => {
                let c = self.counters.entry(arg(0).to_string()).or_insert(0);
                if arg(1) == "-" {
                    *c = 0;
                    return Ok(());
                }
                *c += 1;
                let c = *c;
                self.w(&c.to_string())
            }
            "kbd" => {
                self.w("<kbd>")?;
                self.we(arg(0))?;
                self.w("</kbd>")
            }
            _ => Ok(()),
        }
    }

    fn fn_link(&mut self, n: usize, id: &str) -> fmt::Result {
        self.w(&format!(
            "<sup><a id=\"{id}\" class=\"footref\" href=\"#fn.{n}\" role=\"doc-backlink\">{n}</a></sup>"
        ))
    }

    fn headline_prefix(h: &Headline<'_>) -> String {
        let mut s = String::new();
        if let Some(kw) = &h.todo_keyword {
            let class = match h.todo_type {
                Some(TodoType::Done) => "done",
                _ => "todo",
            };
            let kw = escape(kw);
            s.push_str(&format!("<span class=\"{class} {kw}\">{kw}</span> "));
        }
        if let Some(p) = h.priority {
            s.push_str(&format!("<span class=\"priority\">[#{p}]</span> "));
        }
        s
    }

    fn start_headline(&mut self, h: &Headline<'_>, inline: bool) -> fmt::Result {
        if h.commented || h.tags.iter().any(|t| t == "noexport") {
            self.skip = 1;
            return Ok(());
        }
        let muted = h.footnote_section && !inline;
        if muted {
            self.bufs.push(String::new());
        }
        if !inline {
            self.headline_count += 1;
        }
        self.hstack.push(HInfo {
            id: self.headline_count,
            level: h.level,
            inline,
            archived: h.archived && !inline,
            muted,
            prefix: Self::headline_prefix(h),
            tags: h.tags.iter().map(|t| t.to_string()).collect(),
        });
        if inline {
            self.w("<div class=\"inlinetask\">\n")?;
        }
        Ok(())
    }

    fn start(
        &mut self,
        tag: Tag<'_>,
        kws: &[(String, String)],
        next: Next,
        lead: bool,
    ) -> Result<bool, fmt::Error> {
        if matches!(tag, Tag::Section | Tag::Headline(_) | Tag::Inlinetask(_))
            && self.hstack.last().is_some_and(|h| h.archived)
        {
            self.skip = 1;
            return Ok(false);
        }
        let aff = Aff::from_keywords(kws);
        match tag {
            Tag::Headline(h) => self.start_headline(&h, false)?,
            Tag::Inlinetask(h) => self.start_headline(&h, true)?,
            Tag::Title => {
                if let Some(h) = self.hstack.last() {
                    let inline = h.inline;
                    let prefix = h.prefix.clone();
                    if inline {
                        self.w("<b>")?;
                    } else {
                        self.bufs.push(String::new());
                    }
                    self.w(&prefix)?;
                }
            }
            Tag::Section => {}
            Tag::Paragraph => {
                if aff.is_empty() && lead {
                    self.paras.push(ParaKind::Lead);
                } else if aff.is_empty() {
                    self.paras.push(ParaKind::Plain);
                    self.w("<p>")?;
                } else {
                    self.paras.push(ParaKind::Aff(aff));
                    self.bufs.push(String::new());
                }
            }
            Tag::PlainList { kind } => {
                self.lists.push(kind);
                let t = match kind {
                    ListKind::Unordered => "ul",
                    ListKind::Ordered => "ol",
                    ListKind::Descriptive => "dl",
                };
                self.w(&aff.open(t, None))?;
                self.w("\n")?;
            }
            Tag::Item(item) => {
                let kind = self.lists.last().copied().unwrap_or(ListKind::Unordered);
                let desc = kind == ListKind::Descriptive;
                self.items.push(ItemInfo {
                    desc,
                    checkbox: item.checkbox,
                });
                self.lead = true;
                if desc {
                    if next != Next::StartItemTag {
                        self.w("<dt></dt>\n<dd>")?;
                        if let Some(c) = item.checkbox {
                            self.w(checkbox_html(c))?;
                        }
                    }
                } else {
                    self.w("<li")?;
                    if kind == ListKind::Ordered {
                        if let Some(c) = &item.counter {
                            if c.trim().parse::<i64>().is_ok() {
                                self.w(&format!(" value=\"{}\"", c.trim()))?;
                            }
                        }
                    }
                    self.w(">")?;
                    if let Some(c) = item.checkbox {
                        self.w(checkbox_html(c))?;
                    }
                }
            }
            Tag::ItemTag => {
                let (desc, cb) = self
                    .items
                    .last()
                    .map(|i| (i.desc, i.checkbox))
                    .unwrap_or((false, None));
                if desc {
                    self.w("<dt>")?;
                    if let Some(c) = cb {
                        self.w(checkbox_html(c))?;
                    }
                }
            }
            Tag::Table { .. } | Tag::TableRow { .. } | Tag::TableCell => {}
            Tag::CenterBlock => {
                self.w(&aff.open("div", Some("org-center")))?;
                self.w("\n")?;
            }
            Tag::QuoteBlock => {
                self.w(&aff.open("blockquote", None))?;
                self.w("\n")?;
            }
            Tag::VerseBlock => {
                self.verse += 1;
                self.w(&aff.open("p", Some("verse")))?;
                self.w("\n")?;
            }
            Tag::SpecialBlock { name, .. } => {
                self.w(&aff.open("div", Some(&name)))?;
                self.w("\n")?;
            }
            Tag::SrcBlock {
                language, switches, ..
            } => {
                self.preserve_indent
                    .push(preserves_indent(switches.as_deref()));
                let cap = aff.caption.clone();
                if let Some(c) = &cap {
                    self.w("<div class=\"org-src-container\">\n<label class=\"org-src-name\">")?;
                    self.we(c)?;
                    self.w("</label>\n")?;
                }
                self.src_caption.push(cap.is_some());
                let base = match language.as_deref() {
                    Some(l) if !l.is_empty() => format!("src src-{l}"),
                    _ => "src".to_string(),
                };
                self.w(&aff.open("pre", Some(&base)))?;
                self.bufs.push(String::new());
            }
            Tag::ExampleBlock { switches } => {
                self.preserve_indent
                    .push(preserves_indent(switches.as_deref()));
                self.w(&aff.open("pre", Some("example")))?;
                self.bufs.push(String::new());
            }
            Tag::ExportBlock { backend } => {
                let html = backend.eq_ignore_ascii_case("html");
                self.export_html.push(html);
                if html {
                    self.raw = true;
                } else {
                    self.bufs.push(String::new());
                }
            }
            Tag::CommentBlock | Tag::PropertyDrawer => self.bufs.push(String::new()),
            Tag::Drawer { name } => {
                self.w(&aff.open("div", Some(&format!("drawer {name}"))))?;
                self.w(&format!("\n<h6>{}</h6>\n", escape(&name)))?;
            }
            Tag::DynamicBlock { .. } => {}
            Tag::FootnoteDefinition { label } => {
                self.fn_def_stack.push(label.to_string());
                self.bufs.push(String::new());
                self.lead = true;
            }
            Tag::Bold => self.w("<b>")?,
            Tag::Italic => self.w("<i>")?,
            Tag::Underline => self.w("<span class=\"underline\">")?,
            Tag::StrikeThrough => self.w("<del>")?,
            Tag::Subscript { .. } => self.w("<sub>")?,
            Tag::Superscript { .. } => self.w("<sup>")?,
            Tag::Link(l) => {
                let t = l.link_type.as_ref();
                let anchor = matches!(t, "fuzzy" | "custom-id" | "id" | "coderef" | "radio");
                let href = if anchor {
                    format!("#{}", l.path)
                } else if t == "file" || t.is_empty() {
                    l.path.to_string()
                } else {
                    format!("{}:{}", t, l.path)
                };
                if next == Next::EndLink {
                    let image = matches!(t, "file" | "fuzzy" | "http" | "https")
                        && is_image(&l.path)
                        && l.format != LinkFormat::Angle;
                    if image {
                        let src = if t == "fuzzy" {
                            l.path.to_string()
                        } else {
                            href.clone()
                        };
                        self.w(&format!(
                            "<img src=\"{}\" alt=\"{}\" />",
                            escape(&src),
                            escape(&l.path)
                        ))?;
                    } else {
                        let text = if matches!(t, "file" | "fuzzy" | "radio") {
                            l.path.to_string()
                        } else {
                            l.raw_link.to_string()
                        };
                        self.w(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape(&href),
                            escape(&text)
                        ))?;
                    }
                    return Ok(true);
                }
                self.w(&format!("<a href=\"{}\">", escape(&href)))?;
            }
            Tag::RadioTarget => {
                self.radio_text.push(String::new());
                self.bufs.push(String::new());
            }
            Tag::FootnoteReference { label } => {
                let (n, id) = self.footnote_ref(label.as_deref());
                self.fn_link(n, &id)?;
                self.fn_inline_stack.push(n);
                self.bufs.push(String::new());
            }
            Tag::Citation { .. } => {
                self.cite_first = true;
                self.w("[cite: ")?;
            }
        }
        Ok(false)
    }

    fn end(&mut self, end: TagEnd, next: Next) -> fmt::Result {
        match end {
            TagEnd::Headline | TagEnd::Inlinetask => {
                if let Some(h) = self.hstack.pop() {
                    if h.inline {
                        self.w("</div>\n")?;
                    }
                    if h.muted {
                        self.pop_buf();
                    }
                }
            }
            TagEnd::Title => {
                if let Some(h) = self.hstack.last() {
                    let (inline, level, id) = (h.inline, h.level, h.id);
                    let mut s = String::new();
                    if !h.tags.is_empty() {
                        s.push_str(" <span class=\"tag\">");
                        let spans: Vec<String> = h
                            .tags
                            .iter()
                            .map(|t| {
                                let t = escape(t);
                                format!("<span class=\"{t}\">{t}</span>")
                            })
                            .collect();
                        s.push_str(&spans.join("&#xa0;"));
                        s.push_str("</span>");
                    }
                    self.w(&s)?;
                    if inline {
                        self.w("</b><br />\n")?;
                    } else {
                        let inner = self.pop_buf();
                        self.held = Some(Held {
                            level,
                            id,
                            custom_id: None,
                            inner,
                        });
                    }
                }
            }
            TagEnd::Section => {}
            TagEnd::Paragraph => match self.paras.pop() {
                Some(ParaKind::Aff(aff)) => {
                    let content = self.pop_buf();
                    self.finish_paragraph(&aff, &content)?;
                }
                Some(ParaKind::Lead) => {
                    if next != Next::EndContainer {
                        self.w("\n")?;
                    }
                }
                _ => self.w("</p>\n")?,
            },
            TagEnd::PlainList(kind) => {
                self.lists.pop();
                self.w(match kind {
                    ListKind::Unordered => "</ul>\n",
                    ListKind::Ordered => "</ol>\n",
                    ListKind::Descriptive => "</dl>\n",
                })?;
            }
            TagEnd::Item => {
                let desc = self.items.pop().is_some_and(|i| i.desc);
                self.w(if desc { "</dd>\n" } else { "</li>\n" })?;
            }
            TagEnd::ItemTag => {
                if self.items.last().is_some_and(|i| i.desc) {
                    self.w("</dt>\n<dd>")?;
                }
                self.lead = true;
            }
            TagEnd::Table | TagEnd::TableRow | TagEnd::TableCell => {}
            TagEnd::CenterBlock => self.w("</div>\n")?,
            TagEnd::QuoteBlock => self.w("</blockquote>\n")?,
            TagEnd::VerseBlock => {
                self.verse = self.verse.saturating_sub(1);
                self.w("</p>\n")?;
            }
            TagEnd::SpecialBlock => self.w("</div>\n")?,
            TagEnd::SrcBlock => {
                self.end_verbatim()?;
                if self.src_caption.pop().unwrap_or(false) {
                    self.w("</div>\n")?;
                }
            }
            TagEnd::ExampleBlock => self.end_verbatim()?,
            TagEnd::ExportBlock => {
                if self.export_html.pop().unwrap_or(false) {
                    self.raw = false;
                } else {
                    self.pop_buf();
                }
            }
            TagEnd::CommentBlock | TagEnd::PropertyDrawer => {
                self.pop_buf();
            }
            TagEnd::Drawer => self.w("</div>\n")?,
            TagEnd::DynamicBlock => {}
            TagEnd::FootnoteDefinition => {
                let content = self.pop_buf();
                if let Some(label) = self.fn_def_stack.pop() {
                    self.pending_defs.insert(label, content);
                }
            }
            TagEnd::Bold => self.w("</b>")?,
            TagEnd::Italic => self.w("</i>")?,
            TagEnd::Underline => self.w("</span>")?,
            TagEnd::StrikeThrough => self.w("</del>")?,
            TagEnd::Subscript => self.w("</sub>")?,
            TagEnd::Superscript => self.w("</sup>")?,
            TagEnd::Link => self.w("</a>")?,
            TagEnd::RadioTarget => {
                let content = self.pop_buf();
                let id = self.radio_text.pop().unwrap_or_default();
                self.w(&format!("<a id=\"{}\">{}</a>", escape(&id), content))?;
            }
            TagEnd::FootnoteReference => {
                let content = self.pop_buf();
                if let Some(n) = self.fn_inline_stack.pop() {
                    let e = &mut self.entries[n - 1];
                    if e.def.is_none() {
                        e.def = Some(content);
                    }
                }
            }
            TagEnd::Citation => self.w("]")?,
        }
        Ok(())
    }

    /// Close a src or example block: remove the common indentation of the
    /// buffered (escaped) body unless the block preserves it, then drop one
    /// final newline.
    fn end_verbatim(&mut self) -> fmt::Result {
        let content = self.pop_buf();
        let content = if self.preserve_indent.pop().unwrap_or(false) {
            Cow::Borrowed(content.as_str())
        } else {
            remove_indentation(&content)
        };
        self.w(content.strip_suffix('\n').unwrap_or(&content))?;
        self.w("</pre>\n")
    }

    fn finish_paragraph(&mut self, aff: &Aff, content: &str) -> fmt::Result {
        let trimmed = content.trim();
        let image_only = trimmed.starts_with("<img ")
            && trimmed.ends_with(" />")
            && trimmed.matches('<').count() == 1;
        if image_only {
            let img = format!(
                "{}{}",
                &trimmed[..trimmed.len() - 3],
                format_args!("{} />", aff.attr_string(None, false))
            );
            match &aff.caption {
                Some(cap) => {
                    let id = aff.id.as_ref().map(|i| format!(" id=\"{}\"", escape(i)));
                    self.w(&format!(
                        "<figure{}>\n{img}\n<figcaption>{}</figcaption>\n</figure>\n",
                        id.unwrap_or_default(),
                        escape(cap)
                    ))
                }
                None => {
                    let id = aff.id.as_ref().map(|i| format!(" id=\"{}\"", escape(i)));
                    self.w(&format!("<p{}>{img}</p>\n", id.unwrap_or_default()))
                }
            }
        } else {
            self.w(&aff.open("p", None))?;
            self.w(content)?;
            self.w("</p>\n")
        }
    }

    fn table(
        &mut self,
        kind: TableKind,
        evs: Vec<Event<'_>>,
        kws: &[(String, String)],
    ) -> fmt::Result {
        let aff = Aff::from_keywords(kws);
        if kind == TableKind::TableEl {
            self.w(&aff.open("pre", Some("table-el")))?;
            for e in &evs {
                if let Event::Text(t) = e {
                    self.we(t)?;
                }
            }
            return self.w("</pre>\n");
        }
        // Collect rows: `None` is a rule row.
        type Cell<'e> = Vec<Event<'e>>;
        let mut rows: Vec<Option<Vec<Cell<'_>>>> = Vec::new();
        let mut cell: Option<Cell<'_>> = None;
        for e in evs {
            match e {
                Event::Start(Tag::TableRow { kind }) => {
                    rows.push(if kind == TableRowKind::Rule {
                        None
                    } else {
                        Some(Vec::new())
                    });
                }
                Event::Start(Tag::TableCell) => cell = Some(Vec::new()),
                Event::End(TagEnd::TableCell) => {
                    if let (Some(c), Some(Some(r))) = (cell.take(), rows.last_mut()) {
                        r.push(c);
                    }
                }
                Event::End(TagEnd::TableRow) => {}
                other => {
                    if let Some(c) = &mut cell {
                        c.push(other);
                    }
                }
            }
        }
        let mut groups: Vec<Vec<Vec<Cell<'_>>>> = Vec::new();
        let mut cur: Vec<Vec<Cell<'_>>> = Vec::new();
        let mut saw_rule = false;
        let first_is_rule = matches!(rows.first(), Some(None));
        for r in rows {
            match r {
                Some(r) => cur.push(r),
                None => {
                    saw_rule = true;
                    groups.push(std::mem::take(&mut cur));
                }
            }
        }
        if !cur.is_empty() {
            groups.push(cur);
        }
        let header = saw_rule && !first_is_rule && !groups.is_empty();
        self.w(&aff.open("table", None))?;
        self.w("\n")?;
        if let Some(c) = &aff.caption {
            self.w("<caption>")?;
            self.we(c)?;
            self.w("</caption>\n")?;
        }
        for (i, group) in groups.into_iter().enumerate() {
            if group.is_empty() {
                continue;
            }
            let head = header && i == 0;
            let (sec, cell_tag) = if head {
                ("thead", "th")
            } else {
                ("tbody", "td")
            };
            self.w(&format!("<{sec}>\n"))?;
            for row in group {
                self.w("<tr>\n")?;
                for c in row {
                    self.w(&format!("<{cell_tag}>"))?;
                    self.feed(c.into_iter())?;
                    self.w(&format!("</{cell_tag}>\n"))?;
                }
                self.w("</tr>\n")?;
            }
            self.w(&format!("</{sec}>\n"))?;
        }
        self.w("</table>\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Item, Link};
    use std::borrow::Cow;

    fn t(s: &str) -> Event<'_> {
        Event::Text(Cow::Borrowed(s))
    }
    fn st(tag: Tag<'_>) -> Event<'_> {
        Event::Start(tag)
    }
    fn en(tag: TagEnd) -> Event<'static> {
        Event::End(tag)
    }
    fn kw<'a>(key: &'a str, value: &'a str) -> Event<'a> {
        Event::Keyword(Keyword {
            key: key.into(),
            value: value.into(),
            optional: None,
            affiliated: true,
        })
    }
    fn render(events: Vec<Event<'_>>) -> String {
        let mut s = String::new();
        push_html(&mut s, events.into_iter());
        s
    }
    fn para<'a>(inner: Vec<Event<'a>>) -> Vec<Event<'a>> {
        let mut v = vec![st(Tag::Paragraph)];
        v.extend(inner);
        v.push(en(TagEnd::Paragraph));
        v
    }
    fn headline<'a>(level: usize) -> Headline<'a> {
        Headline {
            level,
            todo_keyword: None,
            todo_type: None,
            priority: None,
            tags: vec![],
            commented: false,
            archived: false,
            footnote_section: false,
            raw_title: "".into(),
        }
    }
    fn item<'a>() -> Item<'a> {
        Item {
            bullet: "- ".into(),
            counter: None,
            checkbox: None,
        }
    }
    fn link<'a>(ty: &'a str, path: &'a str) -> Link<'a> {
        Link {
            link_type: ty.into(),
            path: path.into(),
            raw_link: format!("{ty}:{path}").into(),
            format: LinkFormat::Bracket,
            search_option: None,
            application: None,
        }
    }

    #[test]
    fn paragraph_markup() {
        let ev = para(vec![
            t("a & b "),
            st(Tag::Bold),
            t("x"),
            en(TagEnd::Bold),
            Event::SoftBreak,
            Event::Code("<c>".into()),
            Event::LineBreak,
            st(Tag::Subscript {
                use_brackets: false,
            }),
            t("2"),
            en(TagEnd::Subscript),
        ]);
        assert_eq!(
            render(ev),
            "<p>a &amp; b <b>x</b>\n<code>&lt;c&gt;</code><br />\n<sub>2</sub></p>\n"
        );
    }

    #[test]
    fn headline_todo_priority_tags() {
        let mut h = headline(1);
        h.todo_keyword = Some("TODO".into());
        h.todo_type = Some(TodoType::Todo);
        h.priority = Some(Priority::Letter('A'));
        h.tags = vec!["a".into(), "b".into()];
        let mut ev = vec![
            st(Tag::Headline(h)),
            st(Tag::Title),
            t("Hi"),
            en(TagEnd::Title),
        ];
        ev.push(st(Tag::Section));
        ev.extend(para(vec![t("body")]));
        ev.push(en(TagEnd::Section));
        ev.push(en(TagEnd::Headline));
        assert_eq!(
            render(ev),
            "<h2 id=\"outline-1\"><span class=\"todo TODO\">TODO</span> <span class=\"priority\">[#A]</span> Hi \
<span class=\"tag\"><span class=\"a\">a</span>&#xa0;<span class=\"b\">b</span></span></h2>\n<p>body</p>\n"
        );
    }

    #[test]
    fn done_and_deep_level() {
        let mut h = headline(9);
        h.todo_keyword = Some("DONE".into());
        h.todo_type = Some(TodoType::Done);
        let ev = vec![
            st(Tag::Headline(h)),
            st(Tag::Title),
            en(TagEnd::Title),
            en(TagEnd::Headline),
        ];
        assert_eq!(
            render(ev),
            "<h6 id=\"outline-1\"><span class=\"done DONE\">DONE</span> </h6>\n"
        );
    }

    #[test]
    fn skipped_headlines() {
        let mk = |h: Headline<'static>| {
            vec![
                st(Tag::Headline(h)),
                st(Tag::Title),
                t("secret"),
                en(TagEnd::Title),
                st(Tag::Section),
                en(TagEnd::Section),
                en(TagEnd::Headline),
            ]
        };
        let mut a = headline(1);
        a.commented = true;
        let mut b = headline(1);
        b.tags = vec!["noexport".into()];
        let mut c = headline(1);
        c.footnote_section = true;
        let mut ev = mk(a);
        ev.extend(mk(b));
        ev.extend(mk(c));
        ev.extend(para(vec![t("after")]));
        assert_eq!(render(ev), "<p>after</p>\n");
    }

    #[test]
    fn archived_renders_only_title() {
        let mut h = headline(1);
        h.archived = true;
        let mut ev = vec![
            st(Tag::Headline(h)),
            st(Tag::Title),
            t("Old"),
            en(TagEnd::Title),
        ];
        ev.push(st(Tag::Section));
        ev.extend(para(vec![t("hidden")]));
        ev.push(en(TagEnd::Section));
        ev.push(st(Tag::Headline(headline(2))));
        ev.push(st(Tag::Title));
        ev.push(t("sub"));
        ev.push(en(TagEnd::Title));
        ev.push(en(TagEnd::Headline));
        ev.push(en(TagEnd::Headline));
        ev.extend(para(vec![t("next")]));
        assert_eq!(render(ev), "<h2 id=\"outline-1\">Old</h2>\n<p>next</p>\n");
    }

    #[test]
    fn nested_lists_descriptive_checkbox() {
        let mut it = item();
        it.checkbox = Some(Checkbox::On);
        let mut ordered = item();
        ordered.counter = Some("3".into());
        let ev = vec![
            st(Tag::PlainList {
                kind: ListKind::Unordered,
            }),
            st(Tag::Item(it)),
            st(Tag::Paragraph),
            t("a"),
            en(TagEnd::Paragraph),
            st(Tag::PlainList {
                kind: ListKind::Ordered,
            }),
            st(Tag::Item(ordered)),
            st(Tag::Paragraph),
            t("n"),
            en(TagEnd::Paragraph),
            en(TagEnd::Item),
            en(TagEnd::PlainList(ListKind::Ordered)),
            en(TagEnd::Item),
            en(TagEnd::PlainList(ListKind::Unordered)),
            st(Tag::PlainList {
                kind: ListKind::Descriptive,
            }),
            st(Tag::Item(item())),
            st(Tag::ItemTag),
            t("term"),
            en(TagEnd::ItemTag),
            st(Tag::Paragraph),
            t("def"),
            en(TagEnd::Paragraph),
            en(TagEnd::Item),
            en(TagEnd::PlainList(ListKind::Descriptive)),
        ];
        assert_eq!(
            render(ev),
            "<ul>\n<li><code>[X]</code> a\n<ol>\n<li value=\"3\">n</li>\n</ol>\n</li>\n</ul>\n\
<dl>\n<dt>term</dt>\n<dd>def</dd>\n</dl>\n"
        );
    }

    fn row<'a>(cells: &[&'a str]) -> Vec<Event<'a>> {
        let mut v = vec![st(Tag::TableRow {
            kind: TableRowKind::Standard,
        })];
        for c in cells {
            v.push(st(Tag::TableCell));
            v.push(t(c));
            v.push(en(TagEnd::TableCell));
        }
        v.push(en(TagEnd::TableRow));
        v
    }
    fn rule<'a>() -> Vec<Event<'a>> {
        vec![
            st(Tag::TableRow {
                kind: TableRowKind::Rule,
            }),
            en(TagEnd::TableRow),
        ]
    }
    fn table<'a>(rows: Vec<Vec<Event<'a>>>) -> Vec<Event<'a>> {
        let mut v = vec![st(Tag::Table {
            kind: TableKind::Org,
            formulas: vec![],
        })];
        v.extend(rows.into_iter().flatten());
        v.push(en(TagEnd::Table));
        v
    }

    #[test]
    fn table_with_header_and_groups() {
        let ev = table(vec![row(&["h"]), rule(), row(&["a"]), rule(), row(&["b"])]);
        assert_eq!(
            render(ev),
            "<table>\n<thead>\n<tr>\n<th>h</th>\n</tr>\n</thead>\n\
<tbody>\n<tr>\n<td>a</td>\n</tr>\n</tbody>\n<tbody>\n<tr>\n<td>b</td>\n</tr>\n</tbody>\n</table>\n"
        );
    }

    #[test]
    fn table_without_rule() {
        let ev = table(vec![row(&["a", "b"])]);
        assert_eq!(
            render(ev),
            "<table>\n<tbody>\n<tr>\n<td>a</td>\n<td>b</td>\n</tr>\n</tbody>\n</table>\n"
        );
    }

    #[test]
    fn table_el() {
        let ev = vec![
            st(Tag::Table {
                kind: TableKind::TableEl,
                formulas: vec![],
            }),
            t("+--+\n| <|\n+--+\n"),
            en(TagEnd::Table),
        ];
        assert_eq!(
            render(ev),
            "<pre class=\"table-el\">+--+\n| &lt;|\n+--+\n</pre>\n"
        );
    }

    #[test]
    fn src_block_escaping() {
        let ev = vec![
            st(Tag::SrcBlock {
                language: Some("rust".into()),
                switches: None,
                parameters: None,
            }),
            t("if a < b && c > \"d\" {}\n"),
            en(TagEnd::SrcBlock),
        ];
        assert_eq!(
            render(ev),
            "<pre class=\"src src-rust\">if a &lt; b &amp;&amp; c &gt; &quot;d&quot; {}</pre>\n"
        );
    }

    #[test]
    fn export_and_hidden_blocks() {
        let ev = vec![
            st(Tag::ExportBlock {
                backend: "HTML".into(),
            }),
            t("<b>raw</b>\n"),
            en(TagEnd::ExportBlock),
            st(Tag::ExportBlock {
                backend: "latex".into(),
            }),
            t("\\x\n"),
            en(TagEnd::ExportBlock),
            st(Tag::CommentBlock),
            t("c\n"),
            en(TagEnd::CommentBlock),
            Event::Comment("x".into()),
            st(Tag::PropertyDrawer),
            Event::NodeProperty {
                key: "A".into(),
                value: "b".into(),
            },
            en(TagEnd::PropertyDrawer),
        ];
        assert_eq!(render(ev), "<b>raw</b>\n");
    }

    #[test]
    fn footnotes() {
        let mut ev = para(vec![
            t("a"),
            Event::FootnoteReference("x".into()),
            t("b"),
            st(Tag::FootnoteReference { label: None }),
            t("inline"),
            en(TagEnd::FootnoteReference),
        ]);
        ev.push(st(Tag::FootnoteDefinition { label: "x".into() }));
        ev.extend(para(vec![t("def x")]));
        ev.push(en(TagEnd::FootnoteDefinition));
        let out = render(ev);
        assert_eq!(
            out,
            "<p>a<sup><a id=\"fnr.1\" class=\"footref\" href=\"#fn.1\" role=\"doc-backlink\">1</a></sup>b\
<sup><a id=\"fnr.2\" class=\"footref\" href=\"#fn.2\" role=\"doc-backlink\">2</a></sup></p>\n\
<div id=\"footnotes\"><h2 class=\"footnotes\">Footnotes</h2><div id=\"text-footnotes\">\n\
<div class=\"footdef\"><sup><a id=\"fn.1\" class=\"footnum\" href=\"#fnr.1\">1</a></sup> <div class=\"footpara\">def x</div></div>\n\
<div class=\"footdef\"><sup><a id=\"fn.2\" class=\"footnum\" href=\"#fnr.2\">2</a></sup> <div class=\"footpara\">inline</div></div>\n\
</div></div>\n"
        );
    }

    #[test]
    fn affiliated_table_and_paragraph() {
        let mut ev = vec![
            kw("NAME", "tbl"),
            kw("CAPTION", "A <cap>"),
            kw("ATTR_HTML", ":class foo :style bar"),
        ];
        ev.extend(table(vec![row(&["a"])]));
        assert_eq!(
            render(ev),
            "<table id=\"tbl\" class=\"foo\" style=\"bar\">\n<caption>A &lt;cap&gt;</caption>\n\
<tbody>\n<tr>\n<td>a</td>\n</tr>\n</tbody>\n</table>\n"
        );
        let mut ev = vec![kw("NAME", "p1")];
        ev.extend(para(vec![t("x")]));
        assert_eq!(render(ev), "<p id=\"p1\">x</p>\n");
    }

    #[test]
    fn affiliated_src_and_image() {
        let ev = vec![
            kw("NAME", "s"),
            kw("CAPTION", "Cap"),
            st(Tag::SrcBlock {
                language: Some("sh".into()),
                switches: None,
                parameters: None,
            }),
            t("ls\n"),
            en(TagEnd::SrcBlock),
        ];
        assert_eq!(
            render(ev),
            "<div class=\"org-src-container\">\n<label class=\"org-src-name\">Cap</label>\n\
<pre id=\"s\" class=\"src src-sh\">ls</pre>\n</div>\n"
        );
        let mut ev = vec![kw("CAPTION", "Pic"), kw("ATTR_HTML", ":width 100")];
        ev.extend(para(vec![
            st(Tag::Link(link("file", "a.png"))),
            en(TagEnd::Link),
        ]));
        assert_eq!(
            render(ev),
            "<figure>\n<img src=\"a.png\" alt=\"a.png\" width=\"100\" />\n<figcaption>Pic</figcaption>\n</figure>\n"
        );
    }

    #[test]
    fn verse_and_quote() {
        let ev = vec![
            st(Tag::VerseBlock),
            t("a"),
            Event::SoftBreak,
            t("b"),
            en(TagEnd::VerseBlock),
            st(Tag::QuoteBlock),
            en(TagEnd::QuoteBlock),
        ];
        assert_eq!(
            render(ev),
            "<p class=\"verse\">\na<br />\nb</p>\n<blockquote>\n</blockquote>\n"
        );
    }

    #[test]
    fn links() {
        let ev = para(vec![
            st(Tag::Link(link("https", "//x.org"))),
            t("site"),
            en(TagEnd::Link),
            t(" "),
            st(Tag::Link(link("https", "//x.org"))),
            en(TagEnd::Link),
            t(" "),
            st(Tag::Link(link("fuzzy", "target"))),
            en(TagEnd::Link),
            t(" "),
            st(Tag::Link(link("custom-id", "cid"))),
            t("d"),
            en(TagEnd::Link),
            t(" "),
            st(Tag::Link(link("file", "pic.JPG"))),
            en(TagEnd::Link),
            t(" "),
            st(Tag::Link(link("file", "pic.png"))),
            t("desc"),
            en(TagEnd::Link),
        ]);
        assert_eq!(
            render(ev),
            "<p><a href=\"https://x.org\">site</a> <a href=\"https://x.org\">https://x.org</a> \
<a href=\"#target\">target</a> <a href=\"#cid\">d</a> <img src=\"pic.JPG\" alt=\"pic.JPG\" /> \
<a href=\"pic.png\">desc</a></p>\n"
        );
    }

    #[test]
    fn misc_leaves() {
        let ev = vec![
            Event::Rule,
            Event::Target("tg".into()),
            Event::StatisticsCookie("[1/3]".into()),
            Event::ExportSnippet {
                backend: "html".into(),
                value: "<i>".into(),
            },
            Event::ExportSnippet {
                backend: "latex".into(),
                value: "x".into(),
            },
            Event::Keyword(Keyword {
                key: "TITLE".into(),
                value: "x".into(),
                optional: None,
                affiliated: false,
            }),
            Event::LatexFragment("$a<b$".into()),
            st(Tag::RadioTarget),
            t("rt"),
            en(TagEnd::RadioTarget),
        ];
        assert_eq!(
            render(ev),
            "<hr />\n<a id=\"tg\"></a><code>[1/3]</code><i>\\(a&lt;b\\)<a id=\"rt\">rt</a>"
        );
    }

    #[test]
    fn latex_fragment_delimiters() {
        assert_eq!(
            parse_html("$x^2$ and \\(x^2\\), $$a<b$$ and \\[y\\] cost $5\n"),
            "<p>\\(x^2\\) and \\(x^2\\), \\[a&lt;b\\] and \\[y\\] cost $5</p>\n"
        );
    }

    #[test]
    fn io_writer() {
        let mut buf = Vec::new();
        write_html_io(&mut buf, para(vec![t("x")]).into_iter()).unwrap();
        assert_eq!(buf, b"<p>x</p>\n");
    }

    fn parse_html(src: &str) -> String {
        let mut s = String::new();
        push_html(&mut s, crate::Parser::new(src));
        s
    }

    #[test]
    fn leading_paragraph_has_no_p() {
        assert_eq!(parse_html("- a\n"), "<ul>\n<li>a</li>\n</ul>\n");
        let out = parse_html("- [X] a\n\n  b\n");
        assert!(
            out.starts_with("<ul>\n<li><code>[X]</code> a\n<p>"),
            "{out}"
        );
        assert_eq!(
            parse_html("- t :: d\n"),
            "<dl>\n<dt>t</dt>\n<dd>d</dd>\n</dl>\n"
        );
    }

    #[test]
    fn macros_user_defined() {
        let out = parse_html(
            "#+MACRO: hi Hello $1, $2! $10 $x $$ <b>\n\npara {{{hi(Bob)}}} {{{HI(a\\,b,c)}}}\n",
        );
        assert_eq!(
            out,
            "<p>para Hello Bob, !  $x $$ &lt;b&gt; Hello a,b, c!  $x $$ &lt;b&gt;</p>\n"
        );
    }

    #[test]
    fn macros_builtin() {
        let out = parse_html(
            "#+TITLE: T&C\n#+AUTHOR: Me\n#+DATE: 2024\n\n{{{title}}}|{{{author}}}|{{{date(x)}}}|{{{email}}}|{{{keyword(author)}}}|{{{n}}}{{{n}}}{{{n(b)}}}|{{{kbd(C-c)}}}|{{{property(a)}}}|{{{nope}}}\n",
        );
        assert_eq!(out, "<p>T&amp;C|Me|2024||Me|121|<kbd>C-c</kbd>||</p>\n");
    }

    #[test]
    fn verbatim_trailing_newline_stripped() {
        let ev = vec![Event::FixedWidth("a\nb\n".into())];
        assert_eq!(render(ev), "<pre class=\"example\">a\nb</pre>\n");
        let ev = vec![
            st(Tag::ExampleBlock { switches: None }),
            t("a\nb\n"),
            en(TagEnd::ExampleBlock),
        ];
        assert_eq!(render(ev), "<pre class=\"example\">a\nb</pre>\n");
    }

    #[test]
    fn src_block_in_item_loses_indentation() {
        let out =
            parse_html("- item\n  #+begin_src sh\n  echo nested\n    echo more\n  #+end_src\n");
        assert!(
            out.contains("<pre class=\"src src-sh\">echo nested\n  echo more</pre>\n"),
            "{out}"
        );
    }

    #[test]
    fn example_block_indentation_and_blank_lines() {
        // A blank line, a whitespace-only line shorter than the common
        // indentation (emptied) and one deeper (keeps its extra columns,
        // as in Emacs).
        let out = parse_html("#+begin_example\n    a\n\n  \n      \n    b\n#+end_example\n");
        assert_eq!(out, "<pre class=\"example\">a\n\n\n  \nb</pre>\n");
        // Escaped text is unaffected.
        let ev = vec![
            st(Tag::ExampleBlock { switches: None }),
            t("  <a>\n    & b\n"),
            en(TagEnd::ExampleBlock),
        ];
        assert_eq!(
            render(ev),
            "<pre class=\"example\">&lt;a&gt;\n  &amp; b</pre>\n"
        );
    }

    #[test]
    fn preserve_indentation_switch() {
        let out = parse_html("#+begin_src sh -i\n  keep\n#+end_src\n");
        assert_eq!(out, "<pre class=\"src src-sh\">  keep</pre>\n");
        let out = parse_html("#+begin_src sh -n -i\n  keep\n#+end_src\n");
        assert!(out.contains(">  keep</pre>"), "{out}");
        let out = parse_html("#+begin_example -i\n  keep\n#+end_example\n");
        assert_eq!(out, "<pre class=\"example\">  keep</pre>\n");
        for sw in ["-in", "-ix", "-i2"] {
            let ev = vec![
                st(Tag::SrcBlock {
                    language: Some("sh".into()),
                    switches: Some(sw.into()),
                    parameters: None,
                }),
                t("  strip\n"),
                en(TagEnd::SrcBlock),
            ];
            assert_eq!(
                render(ev),
                "<pre class=\"src src-sh\">strip</pre>\n",
                "{sw}"
            );
        }
        assert!(preserves_indent(Some("-i")));
        assert!(preserves_indent(Some("-n -i -r")));
        assert!(preserves_indent(Some("-i-")));
        assert!(!preserves_indent(Some("-in")));
        assert!(!preserves_indent(Some("-ix -n")));
        assert!(!preserves_indent(None));
    }

    #[test]
    fn remove_indentation_rules() {
        // n = 0: untouched, whitespace-only lines included, no allocation.
        let s = "a\n    \n  b\n";
        assert!(matches!(remove_indentation(s), Cow::Borrowed(x) if x == s));
        // Tabs count up to the next multiple of 8.
        assert_eq!(remove_indentation("\tfoo\n        bar\n"), "foo\nbar\n");
        assert_eq!(remove_indentation("    a\n  b\n"), "  a\nb\n");
        assert_eq!(remove_indentation("  a\n\n  b"), "a\n\nb");
        // Short blank lines are emptied, deeper ones keep the excess.
        assert_eq!(remove_indentation("   a\n \n     \n"), "a\n\n  \n");
        // Whatever follows column n is kept verbatim, tabs included; a tab
        // straddling column n stays (Emacs' default `indent-tabs-mode`).
        assert_eq!(remove_indentation("  a\n  \tb\n"), "a\n\tb\n");
        assert_eq!(remove_indentation("    a\n\tb\n"), "a\n\tb\n");
        // Only blank lines: n is the size plus one, so they are emptied.
        assert_eq!(remove_indentation("  \n \n"), "\n\n");
        // A non-blank line Emacs did not count gives up entirely.
        let s = "    a\n  \x0cb\n";
        assert_eq!(remove_indentation(s), s);
    }

    #[test]
    fn headline_ids() {
        let out = parse_html("* A\n** B\n:PROPERTIES:\n:CUSTOM_ID: bee\n:END:\ntext\n* C\n");
        assert_eq!(
            out,
            "<h2 id=\"outline-1\">A</h2>\n<h3 id=\"bee\">B</h3>\n<p>text</p>\n<h2 id=\"outline-3\">C</h2>\n"
        );
        let out = parse_html("* A\n:PROPERTIES:\n:CUSTOM_ID: x\n:END:\n| a |\n");
        assert!(out.starts_with("<h2 id=\"x\">A</h2>\n<table"), "{out}");
    }
}
