//! The element parser proper: a port of `org-element--parse-elements`,
//! `org-element--current-element` and the element parsers of
//! `org-element.el` (Org 9.8).

use std::borrow::Cow;
use std::ops::Range;
use std::rc::Rc;

use super::text::{eq_ci, is_space, Src};
use super::timestamp::parse_timestamp;
use super::{Element, ElementKind};
use crate::event::{
    Checkbox, ClockStatus, Event, Headline, Item, Keyword, ListKind, TableKind, TableRowKind,
    Timestamp, TimestampKind,
};
use crate::options::Options;
use crate::strings::CowStr;

/// Parsing mode, see `org-element--next-mode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    None,
    FirstSection,
    Section,
    TopComment,
    Planning,
    PropertyDrawer,
    NodeProperty,
    Item,
    TableRow,
}

/// Element type, as far as mode switching and recursion are concerned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Headline,
    Inlinetask,
    Section,
    PlainList,
    Item,
    PropertyDrawer,
    Table,
    Planning,
    Comment,
    Other,
}

fn next_mode(mode: Mode, ty: Ty, parent: bool) -> Mode {
    if parent {
        match ty {
            Ty::Headline => Mode::Section,
            Ty::Section if mode == Mode::FirstSection => Mode::TopComment,
            Ty::Inlinetask => Mode::Planning,
            Ty::PlainList => Mode::Item,
            Ty::PropertyDrawer => Mode::NodeProperty,
            Ty::Section => Mode::Planning,
            Ty::Table => Mode::TableRow,
            _ => Mode::None,
        }
    } else {
        match mode {
            Mode::Item => Mode::Item,
            Mode::NodeProperty => Mode::NodeProperty,
            Mode::Planning if ty == Ty::Planning => Mode::PropertyDrawer,
            Mode::TableRow => Mode::TableRow,
            Mode::TopComment if ty == Ty::Comment => Mode::PropertyDrawer,
            _ => Mode::None,
        }
    }
}

/// One entry of a list structure (`org-list-struct`).
#[derive(Clone, Debug)]
pub(super) struct StructItem {
    pos: usize,
    ind: usize,
    tag: bool,
    end: usize,
}

type Struct = Rc<Vec<StructItem>>;

/// Find the item starting at `pos` in a structure (sorted by position).
fn struct_get(st: &[StructItem], pos: usize) -> Option<&StructItem> {
    st.binary_search_by_key(&pos, |i| i.pos)
        .ok()
        .map(|k| &st[k])
}

/// Result of `org-element--full-item-re` matching.
struct FullItem {
    bullet: Range<usize>,
    counter: Option<Range<usize>>,
    checkbox: Option<Range<usize>>,
    tag: Option<Range<usize>>,
    end: usize,
}

/// Affiliated keywords collected before an element.
struct Affiliated<'a> {
    begin: usize,
    keywords: Vec<(Keyword<'a>, Range<usize>)>,
}

struct Parsed<'a> {
    elem: Element<'a>,
    ty: Ty,
    /// Emacs `:end`, where parsing resumes.
    end: usize,
    /// Contents to parse recursively as elements.
    contents: Option<Range<usize>>,
    structure: Option<Struct>,
}

/// Translation table for affiliated keywords
/// (`org-element-keyword-translation-alist`).
fn translate(key: &str) -> Option<&'static str> {
    Some(match key {
        "DATA" | "LABEL" | "RESNAME" | "SOURCE" | "SRCNAME" | "TBLNAME" => "NAME",
        "RESULT" => "RESULTS",
        "HEADERS" => "HEADER",
        _ => return None,
    })
}

const REGULAR_AFFILIATED: &[&str] = &[
    "DATA", "HEADER", "HEADERS", "LABEL", "NAME", "PLOT", "RESNAME", "RESULT", "SOURCE", "SRCNAME",
    "TBLNAME",
];

/// Upper-case a borrowed string, borrowing when already upper-case.
fn upcase(s: &str) -> CowStr<'_> {
    if s.chars().any(|c| c.is_lowercase()) {
        Cow::Owned(s.to_uppercase())
    } else {
        Cow::Borrowed(s)
    }
}

/// `org-unescape-code-in-string`: remove one comma from lines matching
/// `^[ \t]*,*(,)(?:\*|#\+)`.
pub(super) fn unescape(s: &str) -> CowStr<'_> {
    let needs = s.split('\n').any(|l| escaped_comma(l).is_some());
    if !needs {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for (i, l) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        match escaped_comma(l) {
            Some(k) => {
                out.push_str(&l[..k]);
                out.push_str(&l[k + 1..]);
            }
            None => out.push_str(l),
        }
    }
    Cow::Owned(out)
}

/// Index of the comma to remove in line `l`, if any.
fn escaped_comma(l: &str) -> Option<usize> {
    let b = l.as_bytes();
    let mut i = 0;
    while i < b.len() && matches!(b[i], b' ' | b'\t') {
        i += 1;
    }
    let s = i;
    while i < b.len() && b[i] == b',' {
        i += 1;
    }
    if i == s {
        return None;
    }
    let rest = &b[i..];
    if rest.starts_with(b"*") || rest.starts_with(b"#+") {
        Some(i - 1)
    } else {
        None
    }
}

/// Strip one trailing newline.
fn chop_nl(s: CowStr<'_>) -> CowStr<'_> {
    match s {
        Cow::Borrowed(b) => Cow::Borrowed(b.strip_suffix('\n').unwrap_or(b)),
        Cow::Owned(mut o) => {
            if o.ends_with('\n') {
                o.pop();
            }
            Cow::Owned(o)
        }
    }
}

/// `org-trim` on a borrowed slice.
fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

/// `org-string-nw-p` then `org-trim`.
fn nw_trim(s: &str) -> Option<&str> {
    let t = trim(s);
    if t.chars().any(|c| !is_space(c)) {
        Some(t)
    } else {
        None
    }
}

pub(super) struct ElemParser<'a> {
    pub src: Src<'a>,
    /// Minimum number of stars of an inline task (`org-inlinetask-min-level`,
    /// doubled minus one with `org-odd-levels-only`), if enabled.
    pub task_min: Option<usize>,
    /// Maximum number of stars of a headline ending a section when inline
    /// tasks are enabled (`org-get-limited-outline-regexp`).
    pub outline_max: Option<usize>,
}

impl<'a> ElemParser<'a> {
    pub(super) fn new(src: Src<'a>, opts: &Options, odd_levels: bool) -> Self {
        let (task_min, outline_max) = match opts.inlinetask_min_level {
            None => (None, None),
            Some(min) if odd_levels => (
                Some((2 * min).saturating_sub(1)),
                Some((2 * min.saturating_sub(1)).saturating_sub(1)),
            ),
            Some(min) => (Some(min), Some(min.saturating_sub(1))),
        };
        ElemParser {
            src,
            task_min,
            outline_max,
        }
    }

    fn el(&self, kind: ElementKind<'a>, begin: usize, end: usize) -> Element<'a> {
        Element {
            kind,
            range: begin..self.src.trim_end(begin, end),
            affiliated: Vec::new(),
            children: Vec::new(),
        }
    }

    fn el_aff(&self, kind: ElementKind<'a>, aff: Affiliated<'a>, end: usize) -> Element<'a> {
        let mut e = self.el(kind, aff.begin, end);
        e.affiliated = aff.keywords;
        e
    }

    /// Whether `q` (bol) starts a headline ending a section, i.e. a
    /// headline that is not an inline task.
    fn is_outline(&self, q: usize) -> bool {
        match (self.src.headline_stars(q), self.outline_max) {
            (Some(n), Some(max)) => n <= max,
            (Some(_), None) => true,
            _ => false,
        }
    }

    pub(super) fn parse_elements(
        &self,
        beg: usize,
        end: usize,
        mut mode: Mode,
        structure: Option<&Struct>,
    ) -> Vec<Element<'a>> {
        let mut out = Vec::new();
        let mut pos = beg;
        while pos < end {
            let mut p = self.current_element(pos, end, mode, structure);
            if let Some(c) = p.contents.clone() {
                let child_struct = match p.ty {
                    Ty::PlainList | Ty::Item => p.structure.clone().or_else(|| structure.cloned()),
                    _ => None,
                };
                p.elem.children = self.parse_elements(
                    c.start,
                    c.end,
                    next_mode(mode, p.ty, true),
                    child_struct.as_ref(),
                );
            }
            let next = if p.end > pos {
                p.end
            } else {
                // Safety net: always make progress.
                self.src.next_line(pos).max(pos + 1)
            };
            out.push(p.elem);
            mode = next_mode(mode, p.ty, false);
            pos = next;
        }
        out
    }

    fn leaf(&self, ev: Event<'a>, ty: Ty, begin: usize, end: usize) -> Parsed<'a> {
        Parsed {
            elem: self.el(ElementKind::Leaf(ev), begin, end),
            ty,
            end,
            contents: None,
            structure: None,
        }
    }

    fn leaf_aff(&self, ev: Event<'a>, aff: Affiliated<'a>, end: usize) -> Parsed<'a> {
        Parsed {
            elem: self.el_aff(ElementKind::Leaf(ev), aff, end),
            ty: Ty::Other,
            end,
            contents: None,
            structure: None,
        }
    }

    fn current_element(
        &self,
        p: usize,
        limit: usize,
        mode: Mode,
        structure: Option<&Struct>,
    ) -> Parsed<'a> {
        let s = self.src;
        match mode {
            Mode::Item => {
                if let Some(st) = structure {
                    if struct_get(st, s.bol(p)).is_some() {
                        return self.item(p, limit, st);
                    }
                }
                // Corrupted structure: recompute it.
                let st = Rc::new(self.list_struct(s.bol(p), limit));
                return self.item(p, limit, &st);
            }
            Mode::TableRow => return self.table_row(p),
            Mode::NodeProperty => return self.node_property(p),
            _ => {}
        }
        let at_bol = s.is_bol(p);
        let mut at_task = false;
        if at_bol {
            if let Some(n) = s.headline_stars(p) {
                at_task = true;
                match self.task_min {
                    Some(min) if n >= min => {}
                    _ => return self.headline(p),
                }
            }
        }
        if matches!(mode, Mode::Section | Mode::FirstSection) {
            return self.section(p);
        }
        if at_bol && s.comment_line(p).is_some() {
            return self.comment(p, limit);
        }
        let prev_star = s.prev_line(p).is_some_and(|q| s.at(q) == b'*');
        if at_bol && mode == Mode::Planning && prev_star && s.planning_line(p) {
            return self.planning(p, limit);
        }
        let pd_ok = match mode {
            Mode::Planning => prev_star,
            Mode::PropertyDrawer | Mode::TopComment => true,
            _ => false,
        };
        if at_bol && pd_ok {
            if let Some(end_line) = s.property_drawer(p) {
                return self.property_drawer(p, limit, end_line);
            }
        }
        if !at_bol {
            return self.paragraph(
                Affiliated {
                    begin: p,
                    keywords: Vec::new(),
                },
                p,
                limit,
            );
        }
        if s.clock_line_full(p) {
            return self.clock(p, limit);
        }
        if at_task {
            return self.inlinetask(p, limit);
        }
        // Affiliated keywords.
        let (aff, q) = self.collect_affiliated(p, limit);
        if !aff.keywords.is_empty() && q >= limit {
            return self.keyword(
                Affiliated {
                    begin: aff.begin,
                    keywords: Vec::new(),
                },
                aff.begin,
                limit,
            );
        }
        let p = q;
        // `org-element--current-element-re`.
        if let Some(name) = s.latex_begin(p) {
            return self.latex_environment(aff, p, limit, name);
        }
        if let Some(name) = s.drawer_line(p) {
            return self.drawer(aff, p, limit, name);
        }
        if s.fixed_width_line(p) {
            return self.fixed_width(aff, p, limit);
        }
        if s.dynblock_open(p) {
            return self.dynamic_block(aff, p, limit);
        }
        let i = s.skip_blank(p);
        if s.starts(i, "#+") {
            let j = i + 2;
            if s.starts_ci(j, "BEGIN_") {
                let ns = j + 6;
                let ne = s.non_space_run(ns);
                if ne > ns {
                    return self.block(aff, p, limit, ns..ne);
                }
            }
            if s.starts_ci(j, "CALL:") {
                return self.babel_call(aff, p, limit);
            }
            let run = s.non_space_run(j);
            if s.text[j..run].find(':').is_some_and(|k| k > 0) {
                return self.keyword(aff, p, limit);
            }
        }
        if let Some(label) = s.footnote_def(p) {
            return self.footnote_definition(aff, p, limit, label);
        }
        if s.hrule_line(p) {
            let end = s.after_blank(s.next_line(p), limit);
            return self.leaf_aff(Event::Rule, aff, end);
        }
        if s.starts(p, "%%(") {
            let e = s.eol(p);
            let value = s.s(p..e);
            let end = s.after_blank(s.next_line(p), limit);
            return self.leaf_aff(Event::DiarySexp(value.into()), aff, end);
        }
        if s.table_line(p) || self.tableel_start(p, limit) {
            return self.table(aff, p, limit);
        }
        if s.item_line(p) {
            let st = match structure {
                Some(st) if struct_get(st, p).is_some() => st.clone(),
                _ => Rc::new(self.list_struct(p, limit)),
            };
            return self.plain_list(aff, p, limit, st);
        }
        self.paragraph(aff, p, limit)
    }

    // ------------------------------------------------------------------
    // Affiliated keywords.

    /// `org-element--affiliated-re` at `p`. Returns the official key, the
    /// dual value range and the position where the value begins.
    fn affiliated_at(&self, p: usize) -> Option<(CowStr<'a>, Option<Range<usize>>, usize)> {
        let s = self.src;
        let i = s.skip_blank(p);
        if !s.starts(i, "#+") {
            return None;
        }
        let ks = i + 2;
        let mut ke = ks;
        while ke < s.len() && (s.b[ke].is_ascii_alphanumeric() || matches!(s.b[ke], b'-' | b'_')) {
            ke += 1;
        }
        let raw = s.s(ks..ke);
        let up = raw.to_ascii_uppercase();
        let next = s.at(ke);
        let mut dual = None;
        let colon;
        if up == "CAPTION" || up == "RESULTS" {
            if next == b':' {
                colon = ke;
            } else if next == b'[' {
                // `\[(.*)\]:` greedy: last "]:" on the line.
                let e = s.eol(ke);
                let line = s.s(ke + 1..e);
                let k = line.rfind("]:")?;
                dual = Some(ke + 1..ke + 1 + k);
                colon = ke + 1 + k + 1;
            } else {
                return None;
            }
        } else if REGULAR_AFFILIATED.contains(&up.as_str())
            || (up.starts_with("ATTR_") && up.len() > 5)
        {
            if next != b':' {
                return None;
            }
            colon = ke;
        } else {
            return None;
        }
        let key: CowStr<'a> = match translate(&up) {
            Some(t) => Cow::Borrowed(t),
            None => upcase(raw),
        };
        Some((key, dual, s.skip_blank(colon + 1)))
    }

    fn collect_affiliated(&self, p: usize, limit: usize) -> (Affiliated<'a>, usize) {
        let s = self.src;
        let mut q = p;
        let mut kws = Vec::new();
        while q < limit {
            let Some((key, dual, vbeg)) = self.affiliated_at(q) else {
                break;
            };
            let e = s.eol(q);
            let vend = s.skip_blank_back(e, vbeg).max(vbeg);
            let value = trim(s.s(vbeg.min(vend)..vend));
            let next = s.next_line(q);
            kws.push((
                Keyword {
                    key,
                    value: value.into(),
                    optional: dual.map(|r| Cow::Borrowed(s.s(r))),
                    affiliated: true,
                },
                q..next,
            ));
            if next == q {
                break;
            }
            q = next;
        }
        if !kws.is_empty()
            && (s.blank_from(q)
                || s.comment_line(q).is_some()
                || s.clock_line_prefix(q)
                || s.headline_stars(q).is_some())
        {
            return (
                Affiliated {
                    begin: p,
                    keywords: Vec::new(),
                },
                p,
            );
        }
        (
            Affiliated {
                begin: p,
                keywords: kws,
            },
            q,
        )
    }

    // ------------------------------------------------------------------
    // Headlines and sections.

    fn headline(&self, p: usize) -> Parsed<'a> {
        let s = self.src;
        let stars = s.headline_stars(p).unwrap_or(1);
        let end = s
            .find_line(p + stars, s.len(), |q| {
                s.headline_stars(q).is_some_and(|n| n <= stars)
            })
            .unwrap_or(s.len());
        let after = s.next_line(p);
        let q = s.skip_ws(after, end);
        let contents = if q == end { None } else { Some(s.bol(q)..end) };
        Parsed {
            elem: self.el(
                ElementKind::Headline {
                    data: placeholder_headline(stars),
                    title: p..p,
                },
                p,
                end,
            ),
            ty: Ty::Headline,
            end,
            contents,
            structure: None,
        }
    }

    fn section(&self, p: usize) -> Parsed<'a> {
        let s = self.src;
        let end = s
            .find_line(p, s.len(), |q| q != p && self.is_outline(q))
            .unwrap_or(s.len());
        Parsed {
            elem: self.el(ElementKind::Section, p, end),
            ty: Ty::Section,
            end,
            contents: Some(p..end),
            structure: None,
        }
    }

    fn inlinetask(&self, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let stars = s.headline_stars(p).unwrap_or(1);
        let after = s.next_line(p);
        let task_end = if after > p && s.is_bol(after) {
            s.find_line(after, limit, |q| s.headline_stars(q).is_some())
                .filter(|&q| {
                    let n = s.headline_stars(q).unwrap_or(0);
                    let k = s.skip_blank(q + n + 1);
                    s.starts_ci(k, "END") && s.rest_blank(k + 3)
                })
        } else {
            None
        };
        let contents = task_end.map(|te| {
            let mut q = s.next_line(p);
            while q < s.len() && matches!(s.b[q], b' ' | b'\t' | b'\n') {
                q += 1;
            }
            let cb = s.bol(q).min(te);
            cb..te
        });
        let from = s.next_line(task_end.unwrap_or(p));
        let end = s.after_blank(from, limit);
        Parsed {
            elem: self.el(
                ElementKind::Inlinetask {
                    data: placeholder_headline(stars),
                    title: p..p,
                },
                p,
                end,
            ),
            ty: Ty::Inlinetask,
            end,
            contents,
            structure: None,
        }
    }

    /// End position used by clock and planning parsers.
    fn line_end_blank(&self, p: usize, limit: usize) -> usize {
        let s = self.src;
        let before_blank = s.next_line(p);
        let q = s.skip_ws(before_blank, limit);
        let r = s.skip_blank_back(q, 0);
        if s.is_bol(r) {
            r
        } else {
            s.skip_blank(r)
        }
    }

    fn planning(&self, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let end = self.line_end_blank(p, limit);
        let mut closed = None;
        let mut scheduled = None;
        let mut deadline = None;
        let mut cur = p;
        loop {
            let hay = s.s(cur..end);
            let found = ["CLOSED:", "DEADLINE:", "SCHEDULED:"]
                .iter()
                .filter_map(|k| hay.find(k).map(|i| (i, *k)))
                .min_by_key(|(i, _)| *i);
            let Some((i, k)) = found else { break };
            let after = cur + i + k.len();
            let tpos = s.skip_blank_to(after, end);
            let ts = parse_timestamp(s.text, tpos).map(|(t, _)| t);
            match k {
                "CLOSED:" => closed = ts,
                "DEADLINE:" => deadline = ts,
                _ => scheduled = ts,
            }
            cur = tpos;
        }
        self.leaf(
            Event::Planning {
                closed,
                scheduled,
                deadline,
            },
            Ty::Planning,
            p,
            end,
        )
    }

    fn clock(&self, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let e = s.eol(p);
        let i = s.skip_blank(p);
        let at = s.skip_blank_to(i + 6, e);
        let ts = parse_timestamp(s.text, at).map(|(t, _)| t);
        let duration = s.s(at..e).find("=> ").and_then(|k| {
            let d = s.skip_blank_to(at + k + 3, e);
            let r = s.non_space_run(d).min(e);
            if r > d && s.rest_blank(r) {
                Some(Cow::Borrowed(s.s(d..r)))
            } else {
                None
            }
        });
        let status = if duration.is_some() {
            ClockStatus::Closed
        } else {
            ClockStatus::Running
        };
        let timestamp = ts.unwrap_or(Timestamp {
            kind: TimestampKind::Inactive,
            raw: Cow::Borrowed(""),
            start: None,
            end: None,
            repeater: None,
            warning: None,
        });
        let end = self.line_end_blank(p, limit);
        self.leaf(
            Event::Clock {
                timestamp,
                duration,
                status,
            },
            Ty::Other,
            p,
            end,
        )
    }

    fn property_drawer(&self, p: usize, limit: usize, end_line: usize) -> Parsed<'a> {
        let s = self.src;
        let cb = s.next_line(p);
        let m = s
            .find_line(p, limit, |q| s.end_drawer_line(q))
            .unwrap_or(end_line);
        let contents = if m > cb { Some(cb..m) } else { None };
        let end = s.after_blank(s.next_line(m), limit);
        Parsed {
            elem: self.el(ElementKind::PropertyDrawer, p, end),
            ty: Ty::PropertyDrawer,
            end,
            contents,
            structure: None,
        }
    }

    fn node_property(&self, p: usize) -> Parsed<'a> {
        let s = self.src;
        let e = s.eol(p);
        let i = s.skip_blank(p);
        let (key, value) = if s.at(i) == b':' {
            let run = s.non_space_run(i + 1).min(e);
            let close = s.s(i + 1..run).rfind(':').map(|k| i + 1 + k);
            match close {
                Some(c) if c > i + 1 => {
                    let key = s.s(i + 1..c);
                    let v0 = c + 1;
                    let value = if v0 >= e {
                        ""
                    } else {
                        let vs = s.skip_blank_to(v0, e);
                        let ve = s.skip_blank_back(e, vs);
                        s.s(vs..ve)
                    };
                    (key, value)
                }
                _ => (s.s(i + 1..run), ""),
            }
        } else {
            ("", "")
        };
        let end = (e + 1).min(s.len());
        self.leaf(
            Event::NodeProperty {
                key: key.into(),
                value: value.into(),
            },
            Ty::Other,
            p,
            end,
        )
    }

    fn comment(&self, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let mut parts: Vec<&'a str> = Vec::new();
        let mut q = p;
        while let Some(v) = s.comment_line(q) {
            parts.push(s.s(v.min(s.eol(q))..s.eol(q)));
            q = s.next_line(q);
            if q >= limit {
                break;
            }
        }
        let value: CowStr<'a> = if parts.len() == 1 {
            Cow::Borrowed(parts[0])
        } else {
            Cow::Owned(parts.join("\n"))
        };
        let end = s.after_blank(q, limit);
        self.leaf(Event::Comment(value), Ty::Comment, p, end)
    }

    // ------------------------------------------------------------------
    // Leaf elements accepting affiliated keywords.

    fn keyword(&self, aff: Affiliated<'a>, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let e = s.eol(p);
        let i = s.skip_blank(p) + 2;
        let run = s.non_space_run(i).min(e);
        let colon = s
            .s(i..run)
            .rfind(':')
            .map(|k| i + k)
            .or_else(|| s.s(i..e).find(':').map(|k| i + k))
            .unwrap_or(e);
        let key = upcase(s.s(i..colon));
        let value = trim(s.s((colon + 1).min(e)..e));
        let end = s.after_blank(s.next_line(p), limit);
        self.leaf_aff(
            Event::Keyword(Keyword {
                key,
                value: value.into(),
                optional: None,
                affiliated: false,
            }),
            aff,
            end,
        )
    }

    fn latex_environment(
        &self,
        aff: Affiliated<'a>,
        p: usize,
        limit: usize,
        name: Range<usize>,
    ) -> Parsed<'a> {
        let s = self.src;
        match s.find_latex_end(p, limit, s.s(name)) {
            None => self.paragraph(aff, p, limit),
            Some(m) => {
                let code_end = s.next_line(m);
                let end = s.after_blank(code_end, limit);
                self.leaf_aff(Event::LatexEnvironment(s.s(p..code_end).into()), aff, end)
            }
        }
    }

    fn fixed_width(&self, aff: Affiliated<'a>, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let mut q = p;
        let mut lines = Vec::new();
        while q < limit && s.fixed_width_line(q) {
            let e = s.eol(q);
            let mut k = s.skip_blank(q) + 1;
            if s.at(k) == b' ' && k < e {
                k += 1;
            }
            lines.push(s.s(k.min(e)..e));
            let n = s.next_line(q);
            if n == q {
                break;
            }
            q = n;
        }
        let value: CowStr<'a> = if lines.len() == 1 {
            Cow::Borrowed(lines[0])
        } else {
            Cow::Owned(lines.join("\n"))
        };
        let end = s.after_blank(q, limit);
        self.leaf_aff(Event::FixedWidth(value), aff, end)
    }

    fn babel_call(&self, aff: Affiliated<'a>, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let bb = s.next_line(p);
        let colon = s.s(p..bb).find(':').map(|k| p + k + 1).unwrap_or(p);
        let q = s.skip_blank(colon);
        let mut pt = q;
        while pt < bb && !matches!(s.b[pt], b'[' | b']' | b'(' | b')') {
            pt += 1;
        }
        let call = s.s(q..pt);
        let call = if call.chars().any(|c| !is_space(c)) {
            call
        } else {
            ""
        };
        let inside_header = self
            .paired(&mut pt, b'[', b']')
            .map(|r| Cow::Borrowed(s.s(r)));
        let arguments = self
            .paired(&mut pt, b'(', b')')
            .map(|r| s.s(r))
            .filter(|a| a.chars().any(|c| !is_space(c)))
            .map(Cow::Borrowed);
        let end_header = nw_trim(s.s(pt..s.eol(pt))).map(Cow::Borrowed);
        let end = s.after_blank(s.next_line(pt), limit);
        self.leaf_aff(
            Event::BabelCall {
                call: call.into(),
                inside_header,
                arguments,
                end_header,
            },
            aff,
            end,
        )
    }

    /// `org-element--parse-paired-brackets`: contents between matching
    /// brackets at `*pt`, moving `*pt` past them.
    fn paired(&self, pt: &mut usize, open: u8, close: u8) -> Option<Range<usize>> {
        let s = self.src;
        if s.at(*pt) != open {
            return None;
        }
        let mut depth = 0usize;
        let mut i = *pt;
        while i < s.len() {
            if s.b[i] == open {
                depth += 1;
            } else if s.b[i] == close {
                depth -= 1;
                if depth == 0 {
                    let r = *pt + 1..i;
                    *pt = i + 1;
                    return Some(r);
                }
            }
            i += 1;
        }
        None
    }

    // ------------------------------------------------------------------
    // Greater elements and blocks.

    fn drawer(
        &self,
        aff: Affiliated<'a>,
        p: usize,
        limit: usize,
        name: Range<usize>,
    ) -> Parsed<'a> {
        let s = self.src;
        let from = limit.min(s.eol(p));
        let Some(dend) = s.find_line(from, limit, |q| s.end_drawer_line(q)) else {
            return self.paragraph(aff, p, limit);
        };
        let q = s.skip_ws(s.next_line(p), s.len());
        let cb = s.bol(q);
        let contents = if cb < dend { Some(cb..dend) } else { None };
        let end = s.after_blank(s.next_line(dend), limit);
        Parsed {
            elem: self.el_aff(
                ElementKind::Drawer {
                    name: s.s(name).into(),
                },
                aff,
                end,
            ),
            ty: Ty::Other,
            end,
            contents,
            structure: None,
        }
    }

    fn dynamic_block(&self, aff: Affiliated<'a>, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let bend = s.find_line(p, limit, |q| {
            let i = s.skip_blank(q);
            if !s.starts_ci(i, "#+END") {
                return false;
            }
            let mut j = i + 5;
            if s.at(j) == b':' {
                j += 1;
            }
            s.rest_blank(j)
        });
        let Some(bend) = bend else {
            return self.paragraph(aff, p, limit);
        };
        let e = s.eol(p);
        let i = s.skip_blank(p) + 8;
        let ns = s.skip_blank(i);
        let ne = s.non_space_run(ns).min(e);
        let name = s.s(ns..ne);
        let ws = s.skip_blank_to(ne, e);
        let parameters = if ws > ne {
            if ws < e {
                Some(s.s(ws..e))
            } else {
                // `[ \t]+` backtracks to leave one char to `.+`.
                Some(s.s(e - 1..e))
            }
        } else {
            None
        };
        let cb = s.next_line(p);
        let contents = if cb < bend { Some(cb..bend) } else { None };
        let end = s.after_blank(s.next_line(bend), limit);
        Parsed {
            elem: self.el_aff(
                ElementKind::DynamicBlock {
                    name: name.into(),
                    parameters: parameters.map(Cow::Borrowed),
                },
                aff,
                end,
            ),
            ty: Ty::Other,
            end,
            contents,
            structure: None,
        }
    }

    fn block(&self, aff: Affiliated<'a>, p: usize, limit: usize, name: Range<usize>) -> Parsed<'a> {
        let s = self.src;
        let raw_name = s.s(name.clone());
        let up = raw_name.to_uppercase();
        let search = match up.as_str() {
            "CENTER" | "COMMENT" | "EXAMPLE" | "EXPORT" | "QUOTE" | "SRC" | "VERSE" => up.as_str(),
            _ => raw_name,
        };
        let Some(bend) = s.find_block_end(p, limit, search) else {
            return self.paragraph(aff, p, limit);
        };
        let e = s.eol(p);
        let after_end = s.next_line(bend);
        let end = s.after_blank(after_end, limit);
        let body_start = s.next_line(p).min(bend);
        let greater = |kind: ElementKind<'a>, aff: Affiliated<'a>| {
            let contents = if body_start < bend {
                Some(body_start..bend)
            } else {
                None
            };
            Parsed {
                elem: self.el_aff(kind, aff, end),
                ty: Ty::Other,
                end,
                contents,
                structure: None,
            }
        };
        let raw_body = s.s(body_start..bend);
        let simple = |kind: ElementKind<'a>, aff: Affiliated<'a>| Parsed {
            elem: self.el_aff(kind, aff, end),
            ty: Ty::Other,
            end,
            contents: None,
            structure: None,
        };
        match up.as_str() {
            "CENTER" => greater(ElementKind::CenterBlock, aff),
            "QUOTE" => greater(ElementKind::QuoteBlock, aff),
            "VERSE" => {
                let mut c = body_start..bend;
                if c.end > c.start && s.at(c.end - 1) == b'\n' {
                    c.end -= 1;
                }
                simple(ElementKind::VerseBlock { content: c }, aff)
            }
            "COMMENT" => simple(
                ElementKind::CommentBlock {
                    value: chop_nl(Cow::Borrowed(raw_body)),
                },
                aff,
            ),
            "EXAMPLE" => {
                // `#\+BEGIN_EXAMPLE(?: +(.*))?`
                let k = name.end;
                let switches = if s.at(k) == b' ' {
                    let mut j = k;
                    while s.at(j) == b' ' {
                        j += 1;
                    }
                    Some(Cow::Borrowed(s.s(j.min(e)..e)))
                } else {
                    None
                };
                simple(
                    ElementKind::ExampleBlock {
                        switches,
                        value: chop_nl(unescape(raw_body)),
                    },
                    aff,
                )
            }
            "EXPORT" => {
                // `[ \t]*#\+BEGIN_EXPORT(?:[ \t]+(\S+))?[ \t]*$`
                let k = name.end;
                let ws = s.skip_blank_to(k, e);
                let backend = if ws == e {
                    None
                } else if ws > k {
                    let r = s.non_space_run(ws).min(e);
                    if s.rest_blank(r) {
                        Some(s.s(ws..r))
                    } else {
                        None
                    }
                } else {
                    None
                };
                simple(
                    ElementKind::ExportBlock {
                        backend: backend.map(upcase).unwrap_or(Cow::Borrowed("")),
                        value: chop_nl(unescape(raw_body)),
                    },
                    aff,
                )
            }
            "SRC" => {
                let (language, switches, parameters) = self.src_header(name.end, e);
                simple(
                    ElementKind::SrcBlock {
                        language,
                        switches,
                        parameters,
                        value: chop_nl(unescape(raw_body)),
                    },
                    aff,
                )
            }
            _ => {
                // `[ \t]*#\+BEGIN_(\S+)[ \t]*(.*)[ \t]*$`
                let ps = s.skip_blank_to(name.end, e);
                let parameters = nw_trim(s.s(ps..e)).map(Cow::Borrowed);
                greater(
                    ElementKind::SpecialBlock {
                        name: raw_name.into(),
                        parameters,
                    },
                    aff,
                )
            }
        }
    }

    /// Parse the `#+BEGIN_SRC` line after the block name.
    #[allow(clippy::type_complexity)]
    fn src_header(
        &self,
        k: usize,
        e: usize,
    ) -> (Option<CowStr<'a>>, Option<CowStr<'a>>, Option<CowStr<'a>>) {
        let s = self.src;
        let b = s.b;
        let spaces = |i: usize| {
            let mut j = i;
            while j < e && b[j] == b' ' {
                j += 1;
            }
            j
        };
        let mut t = k;
        // Language: `(?: +(\S+))?`.
        let mut language = None;
        let j = spaces(t);
        if j > t {
            let r = s.non_space_run(j).min(e);
            if r > j {
                language = Some(Cow::Borrowed(s.s(j..r)));
                t = r;
            }
        }
        // Switches.
        let sw_start = t;
        loop {
            let u = spaces(t);
            if u == t || u >= e {
                break;
            }
            let next = if s.starts(u, "-l \"") {
                // `".+"` greedy: last quote, at least one char inside.
                let line = &b[u + 4..e];
                match line.iter().rposition(|&c| c == b'"') {
                    Some(k) if k >= 1 => Some(u + 4 + k + 1),
                    _ => None,
                }
            } else {
                None
            };
            let next = next.or_else(|| {
                if b[u] == b'-' && u + 1 < e && matches!(b[u + 1], b'i' | b'k' | b'r') {
                    Some(u + 2)
                } else if matches!(b[u], b'-' | b'+') && u + 1 < e && b[u + 1] == b'n' {
                    let mut v = u + 2;
                    let w = spaces(v);
                    let d = {
                        let mut d = w;
                        while d < e && b[d].is_ascii_digit() {
                            d += 1;
                        }
                        d
                    };
                    if d > w {
                        v = d;
                    }
                    Some(v)
                } else {
                    None
                }
            });
            match next {
                Some(n) => t = n,
                None => break,
            }
        }
        let switches = if t > sw_start {
            nw_trim(s.s(sw_start..t)).map(Cow::Borrowed)
        } else {
            None
        };
        let parameters = nw_trim(s.s(t..e)).map(Cow::Borrowed);
        (language, switches, parameters)
    }

    fn footnote_definition(
        &self,
        aff: Affiliated<'a>,
        p: usize,
        limit: usize,
        label: Range<usize>,
    ) -> Parsed<'a> {
        let s = self.src;
        let two_blank = |q: usize| {
            if !s.blank_from(q) {
                return false;
            }
            let e1 = s.eol(q);
            if e1 >= s.len() {
                return false;
            }
            let q2 = e1 + 1;
            if !s.blank_from(q2) {
                return false;
            }
            let e2 = s.eol(q2);
            e2 < s.len() && e2 < limit
        };
        let found = s.find_line(s.eol(p), limit, |q| {
            s.headline_stars(q).is_some() || s.footnote_def(q).is_some() || two_blank(q)
        });
        let end = match found {
            None => limit,
            Some(q) if s.at(q) == b'[' && s.footnote_def(q).is_some() => {
                let mut r = s.prev_line(q).unwrap_or(q);
                while r > p && self.affiliated_at(r).is_some() {
                    r = s.prev_line(r).unwrap_or(r);
                }
                s.next_line(r)
            }
            Some(q) if s.headline_stars(q).is_some() => q,
            Some(q) => {
                let r = s.skip_ws(q, limit);
                if r == limit {
                    limit
                } else {
                    s.bol(r)
                }
            }
        };
        let after_label = label.end + 1;
        let q = s.skip_ws(after_label, end);
        let contents = if q == end {
            None
        } else {
            let cb = if s.bol(q) == p { q } else { s.bol(q) };
            let ce = s.contents_end(end, 0);
            Some(cb..ce.max(cb))
        };
        Parsed {
            elem: self.el_aff(
                ElementKind::FootnoteDefinition {
                    label: s.s(label).into(),
                },
                aff,
                end,
            ),
            ty: Ty::Other,
            end,
            contents,
            structure: None,
        }
    }

    // ------------------------------------------------------------------
    // Tables.

    /// The `table.el` heuristic of `org-element--current-element`.
    fn tableel_start(&self, p: usize, limit: usize) -> bool {
        let s = self.src;
        if !s.tableel_rule(p) {
            return false;
        }
        let next = s.next_line(p);
        if next >= limit {
            return false;
        }
        let non_tableel = |q: usize| {
            let i = s.skip_blank(q);
            i >= s.len() || s.b[i] == b'\n' || !matches!(s.b[i], b'+' | b'|' | b' ' | b'\t')
        };
        match s.find_line(s.eol(p), limit, non_tableel) {
            None => {
                // Point moved to LIMIT.
                let last = if s.is_bol(limit) {
                    s.prev_line(limit).unwrap_or(limit)
                } else {
                    s.bol(limit)
                };
                s.tableel_rule(last)
            }
            Some(q) if q == next => false,
            Some(q) => s.prev_line(q).is_some_and(|r| s.tableel_rule(r)),
        }
    }

    fn table(&self, aff: Affiliated<'a>, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let org = s.table_line(p);
        let table_end = s
            .find_line(p, limit, |q| {
                let i = s.skip_blank(q);
                if i >= s.len() || s.b[i] == b'\n' {
                    return true;
                }
                let c = s.b[i];
                if org {
                    c != b'|'
                } else {
                    c != b'|' && c != b'+'
                }
            })
            .unwrap_or(limit);
        let mut q = table_end;
        let mut formulas = Vec::new();
        loop {
            let i = s.skip_blank(q);
            if !s.starts_ci(i, "#+TBLFM:") || s.at(i + 8) != b' ' {
                break;
            }
            let e = s.eol(q);
            let mut j = i + 8;
            while s.at(j) == b' ' {
                j += 1;
            }
            formulas.push(Cow::Borrowed(s.s(j.min(e)..e)));
            let n = s.next_line(q);
            if n == q {
                break;
            }
            q = n;
        }
        // `org-element-table-parser` pushes each line onto `:tblfm`, so the
        // last formula line comes first. Keep Emacs' order.
        formulas.reverse();
        let end = s.after_blank(q, limit);
        let (kind, value, contents) = if org {
            (TableKind::Org, None, Some(p..table_end))
        } else {
            (
                TableKind::TableEl,
                Some(Cow::Borrowed(s.s(p..table_end))),
                None,
            )
        };
        Parsed {
            elem: self.el_aff(
                ElementKind::Table {
                    kind,
                    formulas,
                    value,
                },
                aff,
                end,
            ),
            ty: Ty::Table,
            end,
            contents,
            structure: None,
        }
    }

    fn table_row(&self, p: usize) -> Parsed<'a> {
        let s = self.src;
        let i = s.skip_blank(p);
        let rule = s.at(i) == b'|' && s.at(i + 1) == b'-';
        let end = s.next_line(p);
        let mut el = Element {
            kind: ElementKind::TableRow {
                kind: if rule {
                    TableRowKind::Rule
                } else {
                    TableRowKind::Standard
                },
            },
            range: p..end,
            affiliated: Vec::new(),
            children: Vec::new(),
        };
        if !rule {
            let e = s.eol(p);
            if let Some(k) = s.s(p..e).find('|') {
                let cb = p + k + 1;
                let ce = s.skip_blank_back(e, cb);
                let mut pos = cb;
                while pos < ce {
                    // `[ \t]*(.*?)[ \t]*(?:|\|$)` within cb..ce.
                    let cs = s.skip_blank_to(pos, ce);
                    let mut x = cs;
                    let (content_end, cell_end) = loop {
                        let f = s.skip_blank_to(x, ce);
                        if f >= ce {
                            break (x, ce);
                        }
                        if s.b[f] == b'|' {
                            break (x, f + 1);
                        }
                        x += s.char_at(x).map_or(1, char::len_utf8);
                    };
                    el.children.push(Element {
                        kind: ElementKind::TableCell {
                            content: cs..content_end,
                        },
                        range: pos..cell_end,
                        affiliated: Vec::new(),
                        children: Vec::new(),
                    });
                    if cell_end <= pos {
                        break;
                    }
                    pos = cell_end;
                }
            }
        }
        Parsed {
            elem: el,
            ty: Ty::Other,
            end,
            contents: None,
            structure: None,
        }
    }

    // ------------------------------------------------------------------
    // Lists.

    /// `org-list-full-item-re` at `p` (bol).
    fn full_item(&self, p: usize) -> Option<FullItem> {
        let s = self.src;
        let e = s.eol(p);
        let bs = s.skip_blank(p);
        let mut i = match s.at(bs) {
            b'-' | b'+' | b'*' => bs + 1,
            c if c.is_ascii_digit() => s.digits(bs),
            c if c.is_ascii_alphabetic() => bs + 1,
            _ => return None,
        };
        if s.at(bs).is_ascii_alphanumeric() {
            if !matches!(s.at(i), b'.' | b')') {
                return None;
            }
            i += 1;
        }
        let j = s.skip_blank_to(i, e);
        if j == i && i < e {
            return None;
        }
        i = j;
        let bullet = bs..i;
        // Counter.
        let mut counter = None;
        if s.starts(i, "[@") {
            let try_at = |k: usize| -> Option<(Range<usize>, usize)> {
                let d = s.digits(k);
                if d > k && s.at(d) == b']' {
                    return Some((k..d, d + 1));
                }
                if s.at(k).is_ascii_alphabetic() && s.at(k + 1) == b']' {
                    return Some((k..k + 1, k + 2));
                }
                None
            };
            let r = if s.starts_ci(i + 2, "start:") {
                try_at(i + 8).or_else(|| try_at(i + 2))
            } else {
                try_at(i + 2)
            };
            if let Some((r, after)) = r {
                counter = Some(r);
                i = s.skip_blank_to(after, e);
            }
        }
        // Checkbox.
        let mut checkbox = None;
        if s.at(i) == b'['
            && matches!(s.at(i + 1), b' ' | b'X' | b'x' | b'-')
            && s.at(i + 2) == b']'
        {
            let after = i + 3;
            let j = s.skip_blank_to(after, e);
            if j > after || after >= e {
                checkbox = Some(i..after);
                i = j;
            }
        }
        // Tag: `(.*)[ \t]+::(?:[ \t]+|$)`, greedy.
        let mut tag = None;
        let lb = s.b[i..e].to_vec();
        if lb.len() >= 3 {
            for c in (1..=lb.len() - 2).rev() {
                if lb[c] == b':'
                    && lb[c + 1] == b':'
                    && matches!(lb[c - 1], b' ' | b'\t')
                    && (c + 2 >= lb.len() || matches!(lb[c + 2], b' ' | b'\t'))
                {
                    tag = Some(i..i + c - 1);
                    i = s.skip_blank_to(i + c + 2, e);
                    break;
                }
            }
        }
        Some(FullItem {
            bullet,
            counter,
            checkbox,
            tag,
            end: i,
        })
    }

    /// `org-element--list-struct`.
    fn list_struct(&self, start: usize, limit: usize) -> Vec<StructItem> {
        let s = self.src;
        let mut items: Vec<StructItem> = Vec::new();
        let mut done: Vec<StructItem> = Vec::new();
        let mut p = start;
        let finish = |mut done: Vec<StructItem>, items: Vec<StructItem>, end: Option<usize>| {
            for mut it in items {
                if let Some(e) = end {
                    it.end = e;
                }
                done.push(it);
            }
            done.sort_by_key(|i| i.pos);
            done
        };
        loop {
            if p >= limit {
                let end = s.contents_end(limit, 0);
                return finish(done, items, Some(end));
            }
            // `org-list-end-re`: two blank lines.
            if s.blank_from(p) {
                let e1 = s.eol(p);
                if e1 < s.len() {
                    let q = e1 + 1;
                    if s.blank_from(q) && s.eol(q) < s.len() {
                        return finish(done, items, Some(p));
                    }
                }
            }
            if s.item_line(p) {
                let ind = s.indent_col(p);
                while items.last().is_some_and(|t| ind <= t.ind) {
                    let mut it = items.pop().unwrap();
                    it.end = p;
                    done.push(it);
                }
                let fi = self.full_item(p);
                let tag = fi.as_ref().is_some_and(|f| {
                    f.tag.is_some() && matches!(s.at(f.bullet.start), b'-' | b'+' | b'*')
                });
                items.push(StructItem {
                    pos: p,
                    ind,
                    tag,
                    end: 0,
                });
                p = s.next_line(p);
                continue;
            }
            if s.blank_from(p) {
                let n = s.next_line(p);
                if n == p {
                    // Blank last line without newline: we are at the end.
                    let end = s.contents_end(limit, 0);
                    return finish(done, items, Some(end));
                }
                p = n;
                continue;
            }
            // Inline tasks would be skipped here when
            // `org-inlinetask-max-level` is bound, which stock Org never
            // does: they are handled as ordinary text lines.
            // Text line.
            let ind = s.indent_col(p);
            let end = s.contents_end(p, 0);
            while items.last().is_some_and(|t| ind <= t.ind) {
                let mut it = items.pop().unwrap();
                it.end = end;
                done.push(it);
                if items.is_empty() {
                    return finish(done, items, None);
                }
            }
            // Skip blocks and drawers.
            let i = s.skip_blank(p);
            let mut skipped = None;
            if s.starts_ci(i, "#+BEGIN") {
                let k = i + 7;
                let suffix = if s.at(k) == b':' {
                    Some(":".to_string())
                } else if s.at(k) == b'_' {
                    let r = s.non_space_run(k + 1);
                    if r > k + 1 {
                        Some(s.s(k..r).to_string())
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(suf) = suffix {
                    skipped = s.find_line(p, limit, |q| {
                        let j = s.skip_blank(q);
                        s.starts_ci(j, "#+END")
                            && j + 5 + suf.len() <= s.len()
                            && s.text.is_char_boundary(j + 5 + suf.len())
                            && eq_ci(s.s(j + 5..j + 5 + suf.len()), &suf)
                            && s.rest_blank(j + 5 + suf.len())
                    });
                }
            } else if s.drawer_line(p).is_some() {
                // The search starts on the drawer line itself, which may
                // match when it is a lone `:END:`.
                skipped = s.find_line(p, limit, |q| s.end_drawer_line(q));
            }
            if let Some(q) = skipped {
                p = q;
            }
            let n = s.next_line(p);
            if n == p {
                let end = s.contents_end(limit, 0);
                return finish(done, items, Some(end));
            }
            p = n;
        }
    }

    fn plain_list(&self, aff: Affiliated<'a>, p: usize, limit: usize, st: Struct) -> Parsed<'a> {
        let s = self.src;
        let first = struct_get(&st, p).cloned();
        let kind = if s.at(s.skip_blank(p)).is_ascii_alphanumeric() {
            ListKind::Ordered
        } else if first.as_ref().is_some_and(|f| f.tag) {
            ListKind::Descriptive
        } else {
            ListKind::Unordered
        };
        let mut ce = first.as_ref().map_or(limit, |f| f.end);
        if let Some(f) = &first {
            while let Some(it) = struct_get(&st, ce) {
                if it.ind != f.ind || it.end == ce {
                    break;
                }
                ce = it.end;
            }
        }
        let ce = s.contents_end(ce, 0).max(p).min(limit.max(p));
        let end = s.after_blank_lim(ce, limit);
        Parsed {
            elem: self.el_aff(ElementKind::PlainList { kind }, aff, end),
            ty: Ty::PlainList,
            end,
            contents: Some(p..ce),
            structure: Some(st),
        }
    }

    fn item(&self, p: usize, limit: usize, st: &Struct) -> Parsed<'a> {
        let s = self.src;
        let begin = s.bol(p);
        let fi = self.full_item(begin).unwrap_or(FullItem {
            bullet: begin..begin,
            counter: None,
            checkbox: None,
            tag: None,
            end: begin,
        });
        let sitem = struct_get(st, begin);
        let raw_end = sitem.map_or(limit, |i| i.end);
        let end = limit.min(if s.is_bol(raw_end) {
            raw_end
        } else {
            s.next_line(raw_end)
        });
        let bullet_str = s.s(fi.bullet.clone());
        let ordered = bullet_str.contains(['.', ')']);
        let from = match &fi.tag {
            Some(t) if ordered => t.start,
            _ => fi.end,
        };
        let q = s.skip_ws(from, end);
        let contents = if q >= end {
            None
        } else {
            let cb = if s.bol(q) == begin { q } else { s.bol(q) };
            let ce = s.contents_end(end, 0);
            Some(cb..ce.max(cb))
        };
        let checkbox = fi.checkbox.as_ref().and_then(|r| match s.s(r.clone()) {
            "[ ]" => Some(Checkbox::Off),
            "[X]" => Some(Checkbox::On),
            "[-]" => Some(Checkbox::Trans),
            _ => None,
        });
        let tag = if sitem.is_some_and(|i| i.tag) {
            fi.tag.clone()
        } else {
            None
        };
        let data = Item {
            bullet: bullet_str.into(),
            counter: fi.counter.map(|r| Cow::Borrowed(s.s(r))),
            checkbox,
        };
        Parsed {
            elem: self.el(ElementKind::Item { data, tag }, begin, end),
            ty: Ty::Item,
            end,
            contents,
            structure: Some(st.clone()),
        }
    }

    // ------------------------------------------------------------------
    // Paragraphs.

    /// `org-element-paragraph-separate` at line `q`.
    fn para_separate(&self, q: usize) -> bool {
        let s = self.src;
        if s.headline_stars(q).is_some() || s.footnote_def(q).is_some() || s.starts(q, "%%(") {
            return true;
        }
        let i = s.skip_blank(q);
        let c = s.at(i);
        if i >= s.len() || c == b'\n' || c == b'|' {
            return true;
        }
        if s.tableel_rule(q) {
            return true;
        }
        if c == b'#' {
            let n = s.at(i + 1);
            if n == b' ' || i + 1 >= s.len() || n == b'\n' {
                return true;
            }
            if n == b'+' {
                let j = i + 2;
                if s.starts_ci(j, "BEGIN_") && s.non_space_run(j + 6) > j + 6 {
                    return true;
                }
                // `\S-+(?:\[.*\])?:`
                let run = s.non_space_run(j);
                if run > j {
                    if s.s(j..run).find(':').is_some_and(|k| k >= 1) {
                        return true;
                    }
                    let e = s.eol(j);
                    for (k, ch) in s.s(j..run).char_indices() {
                        if ch == '[' && k >= 1 && s.s(j + k + 1..e).contains("]:") {
                            return true;
                        }
                    }
                }
            }
            return false;
        }
        if c == b':' {
            let n = s.at(i + 1);
            if n == b' ' || i + 1 >= s.len() || n == b'\n' {
                return true;
            }
            return s.drawer_line(q).is_some();
        }
        if s.hrule_line(q) && c == b'-' {
            return true;
        }
        if s.latex_begin(q).is_some() {
            return true;
        }
        if s.clock_line_full(q) {
            return true;
        }
        // Lists: `(?:[-+*]|[0-9]+[.)])(?:[ \t]|$)`.
        let after = match c {
            b'-' | b'+' | b'*' => Some(i + 1),
            d if d.is_ascii_digit() => {
                let e = s.digits(i);
                if matches!(s.at(e), b'.' | b')') {
                    Some(e + 1)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(a) = after {
            if a >= s.len() || matches!(s.at(a), b' ' | b'\t' | b'\n') {
                return true;
            }
        }
        false
    }

    /// Second check of `org-element-paragraph-parser` once a separator line
    /// has been found.
    fn para_confirm(&self, q: usize, limit: usize) -> bool {
        let s = self.src;
        if s.drawer_line(q).is_some() {
            let from = s.next_line(q);
            if from == q || !s.is_bol(from) {
                return false;
            }
            return s.find_line(from, limit, |r| s.end_drawer_line(r)).is_some();
        }
        let i = s.skip_blank(q);
        if s.starts_ci(i, "#+BEGIN_") {
            let ns = i + 8;
            let ne = s.non_space_run(ns);
            if ne > ns {
                return s.find_block_end(q, limit, s.s(ns..ne)).is_some();
            }
        }
        if let Some(name) = s.latex_begin(q) {
            return s.find_latex_end(q, limit, s.s(name)).is_some();
        }
        // `[ \t]*#\+(\S+)\[.*\]:` → only dual keywords separate.
        if s.starts(i, "#+") {
            let j = i + 2;
            let run = s.non_space_run(j);
            let e = s.eol(j);
            let mut best = None;
            for (k, ch) in s.s(j..run).char_indices() {
                if ch == '[' && k >= 1 && s.s(j + k + 1..e).contains("]:") {
                    best = Some(k);
                }
            }
            if let Some(k) = best {
                let name = s.s(j..j + k);
                return name.eq_ignore_ascii_case("CAPTION")
                    || name.eq_ignore_ascii_case("RESULTS");
            }
        }
        true
    }

    fn paragraph(&self, aff: Affiliated<'a>, p: usize, limit: usize) -> Parsed<'a> {
        let s = self.src;
        let cb = p;
        let found = s.find_line(s.eol(p), limit, |q| {
            self.para_separate(q) && self.para_confirm(q, limit)
        });
        let before_blank = found.unwrap_or(limit);
        let ce = s.contents_end(before_blank, cb).max(cb);
        let mut content = cb..ce.min(s.len());
        if content.end > content.start && s.at(content.end - 1) == b'\n' {
            content.end -= 1;
        }
        let end = s.after_blank(before_blank, limit);
        Parsed {
            elem: self.el_aff(ElementKind::Paragraph { content }, aff, end),
            ty: Ty::Other,
            end,
            contents: None,
            structure: None,
        }
    }
}

fn placeholder_headline<'a>(level: usize) -> Headline<'a> {
    Headline {
        level,
        todo_keyword: None,
        todo_type: None,
        priority: None,
        tags: Vec::new(),
        commented: false,
        archived: false,
        footnote_section: false,
        raw_title: Cow::Borrowed(""),
    }
}
