//! Element (block level) parsing.
//!
//! This pass turns the whole document into a tree of [`Element`]s. Containers
//! holding objects (paragraphs, verse blocks, table cells, headline titles,
//! item tags) only record the byte range of their content: objects are parsed
//! lazily by [`crate::object`] when the events are pulled.
//!
//! Reference: `org-element.el`, in particular `org-element--parse-elements`,
//! `org-element--current-element` and the `org-element-*-parser` functions.

mod parse;
mod text;
mod timestamp;

use std::borrow::Cow;
use std::ops::Range;

use crate::event::{
    Event, Headline, Item, Keyword, ListKind, Priority, TableKind, TableRowKind, TodoType,
};
use crate::options::{Options, SubSuperscripts};
use crate::strings::CowStr;

use parse::{ElemParser, Mode};
use text::Src;

/// A parsed element and its children.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Element<'a> {
    pub kind: ElementKind<'a>,
    /// Byte range of the element in the source, *excluding* trailing blank
    /// lines (Emacs' `:begin` to `:end` minus `:post-blank`). For affiliated
    /// keywords, the range starts at the first keyword line.
    pub range: Range<usize>,
    /// Affiliated keywords attached to this element, in source order.
    pub affiliated: Vec<(Keyword<'a>, Range<usize>)>,
    /// Child elements (for greater elements).
    pub children: Vec<Element<'a>>,
}

/// The kind of an element.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ElementKind<'a> {
    /// `title` is the range of the raw title to parse as objects.
    Headline {
        data: Headline<'a>,
        title: Range<usize>,
    },
    Inlinetask {
        data: Headline<'a>,
        title: Range<usize>,
    },
    Section,
    /// `content` is the range to parse as objects.
    Paragraph {
        content: Range<usize>,
    },
    PlainList {
        kind: ListKind,
    },
    /// `tag` is the range of the item tag to parse as objects.
    Item {
        data: Item<'a>,
        tag: Option<Range<usize>>,
    },
    /// `value` is the raw text of a `table.el` table (`None` for Org tables).
    /// `formulas` lists the `#+TBLFM:` lines in source order (Emacs'
    /// `:tblfm` holds them last line first).
    Table {
        kind: TableKind,
        formulas: Vec<CowStr<'a>>,
        value: Option<CowStr<'a>>,
    },
    TableRow {
        kind: TableRowKind,
    },
    TableCell {
        content: Range<usize>,
    },
    CenterBlock,
    QuoteBlock,
    VerseBlock {
        content: Range<usize>,
    },
    SpecialBlock {
        name: CowStr<'a>,
        parameters: Option<CowStr<'a>>,
    },
    /// `value` is the unescaped block body (trailing newline removed).
    SrcBlock {
        language: Option<CowStr<'a>>,
        switches: Option<CowStr<'a>>,
        parameters: Option<CowStr<'a>>,
        value: CowStr<'a>,
    },
    ExampleBlock {
        switches: Option<CowStr<'a>>,
        value: CowStr<'a>,
    },
    ExportBlock {
        backend: CowStr<'a>,
        value: CowStr<'a>,
    },
    CommentBlock {
        value: CowStr<'a>,
    },
    Drawer {
        name: CowStr<'a>,
    },
    PropertyDrawer,
    DynamicBlock {
        name: CowStr<'a>,
        parameters: Option<CowStr<'a>>,
    },
    FootnoteDefinition {
        label: CowStr<'a>,
    },
    /// A leaf element, already in event form: `Keyword`, `Comment`,
    /// `FixedWidth`, `Rule`, `LatexEnvironment`, `BabelCall`, `DiarySexp`,
    /// `Clock`, `Planning`, `NodeProperty`.
    Leaf(Event<'a>),
}

/// Document-wide settings gathered from in-buffer keywords before parsing,
/// merged with the user [`Options`].
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Settings<'a> {
    pub todo_keywords: Vec<CowStr<'a>>,
    pub done_keywords: Vec<CowStr<'a>>,
    pub sub_superscripts: crate::options::SubSuperscripts,
    pub link_abbreviations: Vec<(CowStr<'a>, CowStr<'a>)>,
    /// Radio targets (`<<<text>>>`) found in the document, used to turn
    /// matching plain text into radio links.
    pub radio_targets: Vec<CowStr<'a>>,
}

/// The parsed document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Document<'a> {
    pub elements: Vec<Element<'a>>,
    pub settings: Settings<'a>,
}

/// Parse the whole document into an element tree.
///
/// Line endings: only `\n` is a line separator. A `\r` before it is kept as
/// part of the line, which is what Emacs does when a buffer is visited with
/// a Unix coding system; most patterns still work since `\r` counts as
/// whitespace where Org skips whitespace, but CRLF input is not normalised.
pub(crate) fn parse_document<'a>(text: &'a str, options: &Options) -> Document<'a> {
    let src = Src::new(text);
    // `org-element-org-data-parser`: contents start at the first non blank
    // line.
    let cb = src.bol(src.skip_ws(0, src.len()));
    let parse = |odd: bool| {
        if cb < src.len() {
            ElemParser::new(src, options, odd).parse_elements(
                cb,
                src.len(),
                Mode::FirstSection,
                None,
            )
        } else {
            Vec::new()
        }
    };
    let mut elements = parse(false);
    let (mut settings, odd) = collect_settings(&elements, options);
    if odd && options.inlinetask_min_level.is_some() {
        // `#+STARTUP: odd` changes which headlines are inline tasks.
        elements = parse(true);
        settings = collect_settings(&elements, options).0;
    }
    fix_headlines(src, &mut elements, &settings, options, odd);
    settings.radio_targets = collect_radio_targets(src, &elements);
    Document { elements, settings }
}

// ---------------------------------------------------------------------------
// In-buffer settings (`org-set-regexps-and-options`).

fn walk<'e, 'a>(elements: &'e [Element<'a>], f: &mut impl FnMut(&'e Element<'a>)) {
    for e in elements {
        f(e);
        walk(&e.children, f);
    }
}

/// Name of a TODO keyword without its fast access key: `TODO(t)` -> `TODO`.
fn todo_name(tok: &str) -> &str {
    if tok.ends_with(')') {
        if let Some(k) = tok.find('(') {
            if k + 1 < tok.len() {
                return &tok[..k];
            }
        }
    }
    tok
}

/// Parse a `#+LINK:` value into `(key, replacement)`.
fn link_abbrev(v: &str) -> Option<(&str, &str)> {
    let b = v.as_bytes();
    if b.first() == Some(&b'"') {
        // `\`"\(.+[^\\]\)"[ \t]+\(.+\)`
        for k in (3..b.len()).rev() {
            if b[k] == b'"' && b[k - 1] != b'\\' {
                let mut j = k + 1;
                while j < b.len() && matches!(b[j], b' ' | b'\t') {
                    j += 1;
                }
                if j > k + 1 && j < b.len() && !v[j..].contains('\n') {
                    return Some((&v[1..k], &v[j..]));
                }
            }
        }
    }
    // `\`\(\S-+\)[ \t]+\(.+\)`
    let ke = v.find(|c: char| text::is_space(c)).unwrap_or(v.len());
    if ke == 0 {
        return None;
    }
    let rest = &v[ke..];
    let r = rest.trim_start_matches([' ', '\t']);
    if r.len() == rest.len() || r.is_empty() {
        return None;
    }
    Some((&v[..ke], r))
}

/// Collect the in-buffer settings. Also returns whether
/// `org-odd-levels-only` is set by `#+STARTUP: odd`.
fn collect_settings<'a>(elements: &[Element<'a>], options: &Options) -> (Settings<'a>, bool) {
    let mut typ: Vec<&'a str> = Vec::new();
    let mut todo: Vec<&'a str> = Vec::new();
    let mut seq: Vec<&'a str> = Vec::new();
    let mut opts: Vec<&'a str> = Vec::new();
    let mut links: Vec<&'a str> = Vec::new();
    let mut odd = false;
    walk(elements, &mut |e| {
        if let ElementKind::Leaf(Event::Keyword(k)) = &e.kind {
            if let Cow::Borrowed(v) = k.value {
                match &*k.key {
                    "TYP_TODO" => typ.push(v),
                    "TODO" => todo.push(v),
                    "SEQ_TODO" => seq.push(v),
                    "OPTIONS" => opts.push(v),
                    "LINK" => links.push(v),
                    "STARTUP" => {
                        for opt in v.split_whitespace() {
                            if opt.eq_ignore_ascii_case("odd") {
                                odd = true;
                            } else if opt.eq_ignore_ascii_case("oddeven") {
                                odd = false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    });
    let mut settings = Settings {
        sub_superscripts: options.sub_superscripts,
        ..Settings::default()
    };
    let sequences: Vec<&'a str> = typ.into_iter().chain(todo).chain(seq).collect();
    if sequences.is_empty() {
        settings.todo_keywords = options
            .todo_keywords
            .iter()
            .map(|k| Cow::Owned(k.clone()))
            .collect();
        settings.done_keywords = options
            .done_keywords
            .iter()
            .map(|k| Cow::Owned(k.clone()))
            .collect();
    } else {
        let mut all: Vec<&'a str> = Vec::new();
        let mut done: Vec<&'a str> = Vec::new();
        for value in sequences {
            let toks: Vec<&'a str> = value.split_whitespace().collect();
            let sep = toks.iter().position(|t| *t == "|");
            let names: Vec<&'a str> = toks
                .iter()
                .filter(|t| **t != "|")
                .map(|t| todo_name(t))
                .collect();
            match sep {
                Some(i) => done.extend(
                    toks[i + 1..]
                        .iter()
                        .filter(|t| **t != "|")
                        .map(|t| todo_name(t)),
                ),
                None => done.extend(names.last().copied()),
            }
            all.extend(names);
        }
        settings.todo_keywords = all
            .iter()
            .filter(|k| !done.contains(k))
            .map(|k| Cow::Borrowed(*k))
            .collect();
        settings.done_keywords = done.into_iter().map(Cow::Borrowed).collect();
    }
    for v in opts {
        let mut found = None;
        let mut i = 0;
        while let Some(k) = v[i..].find("^:") {
            let at = i + k + 2;
            let rest = &v[at..];
            if rest.starts_with('t') {
                found = Some(SubSuperscripts::Always);
            } else if rest.starts_with("nil") {
                found = Some(SubSuperscripts::Never);
            } else if rest.starts_with("{}") {
                found = Some(SubSuperscripts::Braces);
            }
            if found.is_some() {
                break;
            }
            i = at;
        }
        if let Some(f) = found {
            settings.sub_superscripts = f;
        }
    }
    let mut abbrevs: Vec<(CowStr<'a>, CowStr<'a>)> = links
        .iter()
        .filter_map(|v| link_abbrev(v))
        .map(|(k, u)| (Cow::Borrowed(k), Cow::Borrowed(u)))
        .collect();
    // Later definitions take precedence (`org-link-abbrev-alist-local` is
    // built in reverse order), then the user options.
    abbrevs.reverse();
    abbrevs.extend(
        options
            .link_abbreviations
            .iter()
            .map(|(k, u)| (Cow::Owned(k.clone()), Cow::Owned(u.clone()))),
    );
    settings.link_abbreviations = abbrevs;
    (settings, odd)
}

// ---------------------------------------------------------------------------
// Headline titles (`org-element--headline-parse-title`).

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '@' | '#' | '%' | ':')
}

/// Compute the headline data and the title range for the headline starting
/// at `p`.
pub(crate) fn headline_info<'a>(
    src: Src<'a>,
    p: usize,
    settings: &Settings<'a>,
    options: &Options,
    odd_levels: bool,
) -> (Headline<'a>, Range<usize>) {
    let e = src.eol(p);
    let mut i = p;
    while src.at(i) == b'*' {
        i += 1;
    }
    // `org-reduced-level`.
    let level = if odd_levels { (i - p) / 2 + 1 } else { i - p };
    i = src.skip_blank_to(i, e);
    // TODO keyword (case sensitive, followed by a space or the end of line).
    let mut todo_keyword = None;
    let mut todo_type = None;
    let mut best: Option<(&CowStr<'a>, TodoType)> = None;
    for (k, ty) in settings
        .todo_keywords
        .iter()
        .map(|k| (k, TodoType::Todo))
        .chain(settings.done_keywords.iter().map(|k| (k, TodoType::Done)))
    {
        if k.is_empty() || !src.starts(i, k) {
            continue;
        }
        let a = i + k.len();
        if (a == e || src.at(a) == b' ') && best.map_or(true, |(b, _)| b.len() < k.len()) {
            best = Some((k, ty));
        }
    }
    if let Some((k, _)) = best {
        let a = i + k.len();
        todo_keyword = Some(Cow::Borrowed(src.s(i..a)));
        todo_type = Some(if settings.done_keywords.iter().any(|d| d == k) {
            TodoType::Done
        } else {
            TodoType::Todo
        });
        i = if src.at(a) == b' ' { a + 1 } else { a };
        i = src.skip_blank_to(i, e);
    }
    // Priority: `.*?\(\[#\(VALUE\)\] ?\)`, VALUE matched case-insensitively.
    let mut priority = None;
    let mut k = i;
    while k + 3 < e + 1 && k < e {
        if src.starts(k, "[#") {
            let v = k + 2;
            let c = src.at(v);
            let mut hit = None;
            if c.is_ascii_alphabetic() && src.at(v + 1) == b']' {
                hit = Some((Priority::Letter(c as char), v + 2));
            } else if c.is_ascii_digit() {
                if src.at(v + 1) == b']' {
                    hit = Some((Priority::Numeric(c - b'0'), v + 2));
                } else if src.at(v + 1).is_ascii_digit() && src.at(v + 2) == b']' {
                    let n = (c - b'0') * 10 + (src.at(v + 1) - b'0');
                    if (10..=64).contains(&n) {
                        hit = Some((Priority::Numeric(n), v + 3));
                    }
                }
            }
            if let Some((ch, after)) = hit {
                priority = Some(ch);
                i = if src.at(after) == b' ' {
                    after + 1
                } else {
                    after
                };
                break;
            }
        }
        k += 1;
    }
    // COMMENT keyword.
    let mut commented = false;
    if src.starts(i, "COMMENT") && (i + 7 == e || src.at(i + 7) == b' ') {
        commented = true;
        i = (i + 7 + usize::from(src.at(i + 7) == b' ')).min(e);
        i = src.skip_blank_to(i, e);
    }
    let title_start = i;
    // Tags.
    let q = src.skip_blank_back(title_start, p);
    let l = src.skip_blank_back(e, q);
    let mut tags = Vec::new();
    let mut title_end = e;
    if let Some(ws) = src.s(q..l).rfind([' ', '\t']) {
        let t = q + ws + 1;
        let token = src.s(t..l);
        if token.len() >= 3
            && token.starts_with(':')
            && token.ends_with(':')
            && token.chars().all(is_tag_char)
        {
            title_end = src.skip_blank_back(t, q);
            tags = token
                .split(':')
                .filter(|t| !t.is_empty())
                .map(Cow::Borrowed)
                .collect();
        }
    }
    let a = src.skip_blank_to(title_start, e);
    let b = src.skip_blank_back(title_end, p).max(a);
    let raw_title = src.s(a..b).trim_matches(|c: char| text::is_space(c));
    let archived = tags.iter().any(|t| t == "ARCHIVE");
    let footnote_section = options
        .footnote_section
        .as_deref()
        .is_some_and(|f| f == raw_title);
    (
        Headline {
            level,
            todo_keyword,
            todo_type,
            priority,
            tags,
            commented,
            archived,
            footnote_section,
            raw_title: Cow::Borrowed(raw_title),
        },
        a..b,
    )
}

fn fix_headlines<'a>(
    src: Src<'a>,
    elements: &mut [Element<'a>],
    settings: &Settings<'a>,
    options: &Options,
    odd: bool,
) {
    for e in elements.iter_mut() {
        match &mut e.kind {
            ElementKind::Headline { data, title } | ElementKind::Inlinetask { data, title } => {
                let (d, t) = headline_info(src, e.range.start, settings, options, odd);
                *data = d;
                *title = t;
            }
            _ => {}
        }
        fix_headlines(src, &mut e.children, settings, options, odd);
    }
}

// ---------------------------------------------------------------------------
// Radio targets.

/// Find `<<<target>>>` in the object containers of the tree.
fn collect_radio_targets<'a>(src: Src<'a>, elements: &[Element<'a>]) -> Vec<CowStr<'a>> {
    let mut found: Vec<&'a str> = Vec::new();
    let mut scan = |r: &Range<usize>| {
        let s = src.s(r.clone());
        let mut i = 0;
        while let Some(k) = s[i..].find("<<<") {
            let st = i + k + 3;
            let inner_end = s[st..]
                .find(['<', '>', '\n', '\r'])
                .map_or(s.len(), |n| st + n);
            let inner = &s[st..inner_end];
            // Cheap stand-in for `org-element-context`: skip targets
            // written inside verbatim or code markup (`=<<<x>>>=`).
            let in_markup = {
                let before = s[..i + k].chars().next_back();
                let after = s[inner_end..].get(3..).and_then(|r| r.chars().next());
                matches!(before, Some('=') | Some('~')) && before == after
            };
            if !inner.is_empty()
                && !in_markup
                && s[inner_end..].starts_with(">>>")
                && !inner.starts_with([' ', '\t'])
                && !inner.ends_with([' ', '\t'])
            {
                if !found.contains(&inner) {
                    found.push(inner);
                }
                i = inner_end + 3;
            } else {
                i = i + k + 1;
            }
        }
    };
    walk(elements, &mut |e| match &e.kind {
        ElementKind::Paragraph { content }
        | ElementKind::VerseBlock { content }
        | ElementKind::TableCell { content } => scan(content),
        ElementKind::Headline { title, .. } | ElementKind::Inlinetask { title, .. } => scan(title),
        ElementKind::Item { tag: Some(t), .. } => scan(t),
        _ => {}
    });
    // `org-update-radio-target-regexp`: most recent first, then longest
    // first (stable sort).
    found.reverse();
    found.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    found.into_iter().map(Cow::Borrowed).collect()
}

#[cfg(test)]
mod tests;
