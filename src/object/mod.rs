//! Object (inline) parsing.
//!
//! Reference: `org-element.el`, in particular `org-element--parse-objects`,
//! `org-element--object-lex`, `org-element-object-restrictions` and the
//! `org-element-*-parser` functions for objects.
//!
//! The parser mirrors Emacs closely: a lexer looks for the next position
//! where `org-element--object-regexp` would match, dispatches on the first
//! characters to the parsers of the object types allowed in the current
//! container, and moves one character forward when they all fail. Containers
//! are then parsed recursively (with an explicit stack, so deeply nested
//! markup cannot overflow the call stack), with the restriction of the object
//! type itself, as `org-element--parse-objects` does.
//!
//! A few conventions differ from the Elisp parse tree:
//!
//! - ranges exclude the trailing blanks (`:post-blank`), which are part of the
//!   following text;
//! - a [`Event::LineBreak`] range includes the trailing spaces and the newline
//!   (as Emacs' `:end`), so no [`Event::SoftBreak`] follows it;
//! - [`crate::Link::raw_link`] is the link as written in the source (Emacs'
//!   `:raw-link` is after escape removal and abbreviation expansion, which is
//!   what `path`/`link_type` are computed from);
//! - prefixes and suffixes of citations and citation references are raw
//!   strings rather than parsed objects;
//! - `Settings::sub_superscripts` is honoured (`org-element` itself ignores the
//!   `^:` option, and the exporters turn the objects back into text): with
//!   `Never` no sub/superscript is produced, with `Braces` only the `_{..}` /
//!   `^{..}` forms are. `Always` is exactly `org-element`'s behaviour.

mod chars;
mod link;
mod misc;
#[cfg(test)]
mod tests;
mod timestamp;

use std::cell::{Cell, RefCell};
use std::ops::Range;

use crate::element::Settings;
use crate::event::{Event, Tag, TagEnd};
use crate::options::SubSuperscripts;
use crate::strings::CowStr;

use chars::{is_alnum, is_line_breakable, is_word, is_ws, starts_with_ci};

/// Which objects are allowed in a given container. Mirrors
/// `org-element-object-restrictions`.
///
/// The *minimal set* is: bold, code, entity, italic, LaTeX fragment,
/// strike-through, subscript, superscript, underline, verbatim. The *standard
/// set* is every object type except table cells and citation references.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub(crate) enum Restriction {
    /// Paragraph, verse block: the full standard set.
    Standard,
    /// Headline and inline task titles: standard set without line breaks.
    HeadlineTitle,
    /// Link description: minimal set plus export snippets, inline babel
    /// calls, inline src blocks, macros and statistics cookies.
    LinkDescription,
    /// Table cell: minimal set plus citations, export snippets, footnote
    /// references, links, macros, radio targets, targets and timestamps.
    TableCell,
    /// Item tag: standard set without line breaks.
    ItemTag,
    /// Parsed keyword values such as `#+CAPTION:`: standard set without
    /// footnote references.
    Keyword,
    /// Inside markup objects (bold, italic, underline, strike-through): the
    /// standard set.
    Markup,
    /// Inside subscript / superscript: the standard set.
    SubSuperscript,
    /// Inside a radio target: the minimal set.
    RadioTarget,
    /// Inside a footnote reference's inline definition: the standard set.
    FootnoteReference,
}

/// A set of object types (bit set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Set(u32);

#[allow(dead_code)]
impl Set {
    const BOLD: u32 = 1 << 0;
    const CITATION: u32 = 1 << 1;
    const CODE: u32 = 1 << 2;
    const ENTITY: u32 = 1 << 3;
    const EXPORT_SNIPPET: u32 = 1 << 4;
    const FOOTNOTE_REFERENCE: u32 = 1 << 5;
    const INLINE_BABEL_CALL: u32 = 1 << 6;
    const INLINE_SRC_BLOCK: u32 = 1 << 7;
    const ITALIC: u32 = 1 << 8;
    const LINE_BREAK: u32 = 1 << 9;
    const LATEX_FRAGMENT: u32 = 1 << 10;
    const LINK: u32 = 1 << 11;
    const MACRO: u32 = 1 << 12;
    const RADIO_TARGET: u32 = 1 << 13;
    const STATISTICS_COOKIE: u32 = 1 << 14;
    const STRIKE_THROUGH: u32 = 1 << 15;
    const SUBSCRIPT: u32 = 1 << 16;
    const SUPERSCRIPT: u32 = 1 << 17;
    const TARGET: u32 = 1 << 18;
    const TIMESTAMP: u32 = 1 << 19;
    const UNDERLINE: u32 = 1 << 20;
    const VERBATIM: u32 = 1 << 21;

    const MINIMAL: u32 = Self::BOLD
        | Self::CODE
        | Self::ENTITY
        | Self::ITALIC
        | Self::LATEX_FRAGMENT
        | Self::STRIKE_THROUGH
        | Self::SUBSCRIPT
        | Self::SUPERSCRIPT
        | Self::UNDERLINE
        | Self::VERBATIM;
    const STANDARD: u32 = (1 << 22) - 1;

    fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
}

impl Restriction {
    fn set(self) -> Set {
        Set(match self {
            Restriction::Standard
            | Restriction::Markup
            | Restriction::SubSuperscript
            | Restriction::FootnoteReference => Set::STANDARD,
            Restriction::HeadlineTitle | Restriction::ItemTag => Set::STANDARD & !Set::LINE_BREAK,
            Restriction::Keyword => Set::STANDARD & !Set::FOOTNOTE_REFERENCE,
            Restriction::LinkDescription => {
                Set::MINIMAL
                    | Set::EXPORT_SNIPPET
                    | Set::INLINE_BABEL_CALL
                    | Set::INLINE_SRC_BLOCK
                    | Set::MACRO
                    | Set::STATISTICS_COOKIE
            }
            Restriction::TableCell => {
                Set::MINIMAL
                    | Set::CITATION
                    | Set::EXPORT_SNIPPET
                    | Set::FOOTNOTE_REFERENCE
                    | Set::LINK
                    | Set::MACRO
                    | Set::RADIO_TARGET
                    | Set::TARGET
                    | Set::TIMESTAMP
            }
            Restriction::RadioTarget => Set::MINIMAL,
        })
    }
}

/// Parse the objects in `text[range]`, appending `(event, range)` pairs to
/// `out`. Ranges are absolute byte offsets in `text`.
///
/// Newlines are emitted as [`Event::SoftBreak`]; text is otherwise emitted as
/// borrowed [`Event::Text`] runs.
pub(crate) fn parse_objects<'a>(
    text: &'a str,
    range: Range<usize>,
    restriction: Restriction,
    settings: &Settings<'a>,
    out: &mut Vec<(Event<'a>, Range<usize>)>,
) {
    let end = range.end.min(text.len());
    let start = range.start.min(end);
    if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        debug_assert!(false, "parse_objects: range not on char boundaries");
        return;
    }
    let parser = ObjectParser::new(text, settings);
    let mut stack = vec![Frame {
        start,
        end,
        pos: start,
        text_from: start,
        mode: Mode::Objects(restriction.set()),
        close: None,
        cache: FrameCache::default(),
    }];
    while let Some(frame) = stack.last_mut() {
        let sc = Scanner {
            p: &parser,
            start: frame.start,
            end: frame.end,
            cache: &frame.cache,
        };
        let next = if frame.pos >= frame.end {
            None
        } else {
            match frame.mode {
                Mode::Objects(set) => sc.lex(frame.pos, set),
                Mode::CitationReferences => sc.citation_reference(frame.pos),
            }
        };
        let obj = match next {
            // A line break at the very end of the container may reach one
            // byte past it, over the container's trailing newline.
            Some(o)
                if o.begin >= frame.pos
                    && o.after > frame.pos
                    && (o.end <= o.after || (o.after == frame.end && o.end == frame.end + 1))
                    && o.after <= frame.end =>
            {
                o
            }
            _ => {
                push_text(text, frame.text_from..frame.end, out);
                let close = frame.close.take();
                stack.pop();
                if let Some((end, range)) = close {
                    out.push((Event::End(end), range));
                }
                continue;
            }
        };
        push_text(text, frame.text_from..obj.begin, out);
        // Parsing resumes after the trailing blanks, but these belong to the
        // following text.
        frame.pos = obj.after;
        frame.text_from = obj.end.min(frame.end);
        let range = obj.begin..obj.end;
        match obj.kind {
            Kind::Leaf(ev) => out.push((ev, range)),
            Kind::Container {
                tag,
                contents,
                restriction,
            } => {
                let end = tag.to_end();
                out.push((Event::Start(tag), range.clone()));
                match contents {
                    Some(c) if c.start >= range.start && c.end <= range.end && c.start < c.end => {
                        stack.push(Frame {
                            start: c.start,
                            end: c.end,
                            pos: c.start,
                            text_from: c.start,
                            mode: Mode::Objects(restriction.set()),
                            close: Some((end, range)),
                            cache: FrameCache::default(),
                        });
                    }
                    _ => out.push((Event::End(end), range)),
                }
            }
            Kind::Citation { tag, contents } => {
                out.push((Event::Start(tag), range.clone()));
                stack.push(Frame {
                    start: contents.start,
                    end: contents.end,
                    pos: contents.start,
                    text_from: contents.start,
                    mode: Mode::CitationReferences,
                    close: Some((TagEnd::Citation, range)),
                    cache: FrameCache::default(),
                });
            }
        }
    }
}

/// Emit `text[range]` as `Text` runs separated by `SoftBreak`s.
fn push_text<'a>(text: &'a str, range: Range<usize>, out: &mut Vec<(Event<'a>, Range<usize>)>) {
    let mut pos = range.start;
    while pos < range.end {
        let nl = text[pos..range.end].find('\n').map(|i| pos + i);
        let stop = nl.unwrap_or(range.end);
        if stop > pos {
            out.push((Event::Text(CowStr::Borrowed(&text[pos..stop])), pos..stop));
        }
        match nl {
            Some(n) => {
                out.push((Event::SoftBreak, n..n + 1));
                pos = n + 1;
            }
            None => break,
        }
    }
}

enum Mode {
    Objects(Set),
    CitationReferences,
}

struct Frame {
    start: usize,
    end: usize,
    /// Where the lexer resumes.
    pos: usize,
    /// Start of the pending text (before `pos` when the previous object had
    /// trailing blanks).
    text_from: usize,
    mode: Mode,
    close: Option<(TagEnd, Range<usize>)>,
    cache: FrameCache,
}

/// A parsed object.
struct Obj<'a> {
    begin: usize,
    /// End, excluding trailing blanks.
    end: usize,
    /// End, including trailing blanks (where parsing resumes).
    after: usize,
    kind: Kind<'a>,
}

enum Kind<'a> {
    Leaf(Event<'a>),
    Container {
        tag: Tag<'a>,
        contents: Option<Range<usize>>,
        restriction: Restriction,
    },
    Citation {
        tag: Tag<'a>,
        contents: Range<usize>,
    },
}

/// A radio target, pre-processed for matching.
struct Radio {
    /// Characters, lower-cased; `None` stands for a run of spaces, which
    /// matches one or more whitespace characters.
    pieces: Vec<Option<char>>,
}

/// Document-wide data shared by all containers.
struct ObjectParser<'a, 's> {
    text: &'a str,
    settings: &'s Settings<'a>,
    radios: Vec<Radio>,
}

impl<'a, 's> ObjectParser<'a, 's> {
    fn new(text: &'a str, settings: &'s Settings<'a>) -> Self {
        let mut targets: Vec<&str> = Vec::new();
        for t in &settings.radio_targets {
            let t: &str = t;
            if !t.is_empty() && !targets.contains(&t) {
                targets.push(t);
            }
        }
        // `org-update-radio-target-regexp' tries longer targets first.
        targets.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
        let radios = targets
            .into_iter()
            .map(|t| {
                let mut pieces = Vec::new();
                for c in t.chars() {
                    if c == ' ' {
                        if pieces.last() != Some(&None) {
                            pieces.push(None);
                        }
                    } else {
                        pieces.push(Some(c));
                    }
                }
                Radio { pieces }
            })
            .collect();
        ObjectParser {
            text,
            settings,
            radios,
        }
    }
}

/// Caches of forward searches in a container. A search from `from` that
/// found its leftmost match at `m` (or nothing) gives the same answer for any
/// start in `from..=m`, which keeps repeated failing searches (many openings
/// without a closing delimiter) linear.
#[derive(Default)]
struct FrameCache {
    /// Last radio target search: `(from, result)`.
    radio: Cell<Option<(usize, Option<RadioMatch>)>>,
    /// Last `find` per needle: `(needle, from, result)`.
    finds: RefCell<Vec<(&'static str, usize, Option<usize>)>>,
    /// Last emphasis closing search per marker (`*/_+~=`): `(from,
    /// Some((match start, marker position)))`.
    emphasis: [Cell<Option<EmphasisSearch>>; 6],
}

#[derive(Clone, Copy, Debug)]
struct RadioMatch {
    /// Start of the whole match (including the character before the
    /// target).
    start: usize,
    /// The target itself (group 1).
    target: (usize, usize),
    /// End of the whole match (including the character after the target).
    end: usize,
}

/// A view on the current container. Emacs narrows the buffer to the
/// container, so nothing outside `start..end` is visible: line beginnings,
/// line ends, `char-before`, word boundaries and searches all stop at the
/// container boundaries.
/// Cached result of a forward search for an emphasis closer: the position the
/// search started from and the match found, if any.
type EmphasisSearch = (usize, Option<(usize, usize)>);

#[derive(Clone, Copy)]
struct Scanner<'p, 'a, 's> {
    p: &'p ObjectParser<'a, 's>,
    start: usize,
    end: usize,
    cache: &'p FrameCache,
}

impl<'p, 'a, 's> Scanner<'p, 'a, 's> {
    fn text(&self) -> &'a str {
        self.p.text
    }

    fn bytes(&self) -> &'a [u8] {
        self.p.text.as_bytes()
    }

    /// The character at `p`, if inside the container.
    fn ch(&self, p: usize) -> Option<char> {
        if p < self.end {
            self.p.text[p..self.end].chars().next()
        } else {
            None
        }
    }

    /// The byte at `p`, if inside the container.
    fn byte(&self, p: usize) -> Option<u8> {
        if p < self.end {
            Some(self.bytes()[p])
        } else {
            None
        }
    }

    /// The character before `p`, if inside the container.
    fn prev(&self, p: usize) -> Option<char> {
        if p > self.start {
            self.p.text[self.start..p].chars().next_back()
        } else {
            None
        }
    }

    fn bol(&self, p: usize) -> bool {
        p <= self.start || self.bytes()[p - 1] == b'\n'
    }

    fn eol(&self, p: usize) -> bool {
        p >= self.end || self.bytes()[p] == b'\n'
    }

    /// `\<`: beginning of a word. Only used before ASCII letters (link
    /// types, `call_`, `src_`): Emacs also sees a word boundary between
    /// word characters of different scripts, e.g. `中http://x`.
    fn word_start(&self, p: usize) -> bool {
        self.ch(p).is_some_and(is_word)
            && !self
                .prev(p)
                .is_some_and(|c| is_word(c) && chars::is_latin_script(c))
    }

    /// The rest of the container from `p`.
    fn rest(&self, p: usize) -> &'a str {
        if p < self.end {
            &self.p.text[p..self.end]
        } else {
            ""
        }
    }

    fn starts_with(&self, p: usize, s: &str) -> bool {
        self.rest(p).starts_with(s)
    }

    /// Find `needle` at or after `p`, within the container.
    fn find(&self, p: usize, needle: &'static str) -> Option<usize> {
        let mut finds = self.cache.finds.borrow_mut();
        let slot = finds.iter_mut().find(|(n, _, _)| *n == needle);
        if let Some((_, from, res)) = &slot {
            if *from <= p {
                match *res {
                    None => return None,
                    Some(m) if m >= p => return Some(m),
                    _ => {}
                }
            }
        }
        let res = self.rest(p).find(needle).map(|i| p + i);
        match slot {
            Some(entry) => *entry = (needle, p, res),
            None => finds.push((needle, p, res)),
        }
        res
    }

    /// Skip spaces and tabs.
    fn skip_blank(&self, mut p: usize) -> usize {
        while matches!(self.byte(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        p
    }

    fn slice(&self, r: Range<usize>) -> &'a str {
        &self.p.text[r]
    }

    fn borrow(&self, r: Range<usize>) -> CowStr<'a> {
        CowStr::Borrowed(&self.p.text[r])
    }

    /// Build an object, computing the trailing blanks.
    fn obj(&self, begin: usize, end: usize, kind: Kind<'a>) -> Obj<'a> {
        Obj {
            begin,
            end,
            after: self.skip_blank(end),
            kind,
        }
    }

    /// Position of the bracket closing the one at `open` (which must be
    /// `o`), counting only `o`/`c` characters, as `scan-lists` does with
    /// `org-element--pair-*-table`. Returns the position *after* the closing
    /// bracket.
    fn balanced(&self, open: usize, o: u8, c: u8) -> Option<usize> {
        if self.byte(open) != Some(o) {
            return None;
        }
        let mut depth = 0usize;
        for (i, &b) in self.bytes()[open..self.end].iter().enumerate() {
            if b == o {
                depth += 1;
            } else if b == c {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
        }
        None
    }

    // ---- Lexer --------------------------------------------------------

    /// Find the next object at or after `pos` (`org-element--object-lex`).
    fn lex(&self, pos: usize, set: Set) -> Option<Obj<'a>> {
        let radio = if set.has(Set::LINK) && !self.p.radios.is_empty() {
            self.radio_limit(pos)
        } else {
            None
        };
        // The object regexp is searched with a bound one character after the
        // beginning of the next radio link.
        let limit = radio.map(|t| t + self.ch(t).map_or(1, char::len_utf8));
        let mut p = pos;
        while p < self.end {
            if limit.is_some_and(|l| p >= l) {
                break;
            }
            if let Some(o) = self.try_at(p, set, limit) {
                return Some(o);
            }
            p += self.ch(p).map_or(1, char::len_utf8);
        }
        radio.and_then(|t| self.link(t))
    }

    /// Next radio link start for the lexer bound.
    fn radio_limit(&self, pos: usize) -> Option<usize> {
        let from = if self.bol(pos) {
            pos
        } else {
            pos - self.prev(pos).map_or(1, char::len_utf8)
        };
        let mut m = self.radio_search(from)?;
        if m.target.0 < pos && !(self.bol(from) && m.target.1 == pos) {
            // Emacs would signal an error (search bound before point);
            // ignore that target instead.
            m = self.radio_search(pos)?;
        }
        // Do not match again a one-character radio link ending at `pos`,
        // which can only happen at the beginning of a line.
        if !self.bol(pos) && self.bol(from) && m.target.1 == pos {
            return self.radio_search(m.end).map(|m| m.target.0);
        }
        Some(m.target.0)
    }

    /// Leftmost match of `org-target-link-regexp` starting at or after
    /// `from`.
    fn radio_search(&self, from: usize) -> Option<RadioMatch> {
        let cached = self.cache.radio.get();
        if let Some((f, res)) = cached {
            if f <= from {
                match res {
                    None => return None,
                    Some(m) if m.start >= from => return Some(m),
                    _ => {}
                }
            }
        }
        let mut m = from;
        let mut res = None;
        while m < self.end {
            if let Some(r) = self.radio_at(m) {
                res = Some(r);
                break;
            }
            m += self.ch(m).map_or(1, char::len_utf8);
        }
        self.cache.radio.set(Some((from, res)));
        res
    }

    /// Match `org-target-link-regexp` exactly at `m`:
    /// `\(?:^\|[^[:alnum:]]\|\c|\)\(TARGETS\)\(?:$\|[^[:alnum:]]\|\c|\)`.
    fn radio_at(&self, m: usize) -> Option<RadioMatch> {
        let mut starts = [None, None];
        if self.bol(m) {
            starts[0] = Some(m);
        }
        if let Some(c) = self.ch(m) {
            if !is_alnum(c) || is_line_breakable(c) {
                starts[1] = Some(m + c.len_utf8());
            }
        }
        for s in starts.into_iter().flatten() {
            for r in &self.p.radios {
                if let Some(e) = self.radio_target_at(s, r) {
                    let after = if self.eol(e) {
                        Some(e)
                    } else {
                        self.ch(e)
                            .filter(|&c| !is_alnum(c) || is_line_breakable(c))
                            .map(|c| e + c.len_utf8())
                    };
                    if let Some(end) = after {
                        return Some(RadioMatch {
                            start: m,
                            target: (s, e),
                            end,
                        });
                    }
                }
            }
        }
        None
    }

    /// Match one radio target at `s`, case-insensitively, spaces matching
    /// any run of whitespace. Returns the end of the match.
    fn radio_target_at(&self, s: usize, r: &Radio) -> Option<usize> {
        let mut p = s;
        for piece in &r.pieces {
            match piece {
                Some(pc) => {
                    let c = self.ch(p)?;
                    if !chars::eq_fold(c, *pc) {
                        return None;
                    }
                    p += c.len_utf8();
                }
                None => {
                    let mut n = 0;
                    while let Some(c) = self.ch(p).filter(|&c| is_ws(c)) {
                        p += c.len_utf8();
                        n += 1;
                    }
                    if n == 0 {
                        return None;
                    }
                }
            }
        }
        Some(p)
    }

    /// Try the parsers relevant at `p`, if `org-element--object-regexp`
    /// matches there.
    fn try_at(&self, p: usize, set: Set, limit: Option<usize>) -> Option<Obj<'a>> {
        let b = self.byte(p)?;
        if !b.is_ascii() {
            return None;
        }
        let fits = |len: usize| limit.map_or(true, |l| p + len <= l);
        let next = self.ch(p + 1);
        let nonspace_next = next.is_some_and(|c| !is_ws(c));
        let subsup = self.p.settings.sub_superscripts;
        match b {
            b'_' => {
                if !nonspace_next || !fits(2) {
                    return None;
                }
                (if set.has(Set::UNDERLINE) {
                    self.emphasis(p, b'_')
                } else {
                    None
                })
                .or_else(|| {
                    if set.has(Set::SUBSCRIPT) && subsup != SubSuperscripts::Never {
                        self.subsup(p, false)
                    } else {
                        None
                    }
                })
            }
            b'^' => {
                let ok = next.is_some_and(|c| {
                    matches!(c, '-' | '{' | '(' | '*' | '+' | '.' | ',') || is_alnum(c)
                });
                if ok && fits(2) && set.has(Set::SUPERSCRIPT) && subsup != SubSuperscripts::Never {
                    self.subsup(p, true)
                } else {
                    None
                }
            }
            b'*' | b'/' | b'~' | b'=' | b'+' => {
                if !nonspace_next || !fits(2) {
                    return None;
                }
                let bit = match b {
                    b'*' => Set::BOLD,
                    b'/' => Set::ITALIC,
                    b'~' => Set::CODE,
                    b'=' => Set::VERBATIM,
                    _ => Set::STRIKE_THROUGH,
                };
                if set.has(bit) {
                    self.emphasis(p, b)
                } else {
                    None
                }
            }
            b'@' => {
                if self.starts_with(p, "@@") && fits(2) && set.has(Set::EXPORT_SNIPPET) {
                    self.export_snippet(p)
                } else {
                    None
                }
            }
            b'{' => {
                if self.starts_with(p, "{{{") && fits(3) && set.has(Set::MACRO) {
                    self.macro_(p)
                } else {
                    None
                }
            }
            b'$' => {
                if fits(1) && set.has(Set::LATEX_FRAGMENT) {
                    self.latex_fragment(p)
                } else {
                    None
                }
            }
            b'<' => {
                let rest = self.rest(p + 1);
                if rest.starts_with('<') {
                    if !fits(2) {
                        return None;
                    }
                    (if set.has(Set::RADIO_TARGET) {
                        self.radio_target(p)
                    } else {
                        None
                    })
                    .or_else(|| {
                        if set.has(Set::TARGET) {
                            self.target(p)
                        } else {
                            None
                        }
                    })
                } else {
                    let len = if rest.starts_with("%%") {
                        3
                    } else if rest.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                        2
                    } else {
                        1 + link::type_prefix(rest, self.p.settings)?.len()
                    };
                    if !fits(len) {
                        return None;
                    }
                    (if set.has(Set::TIMESTAMP) {
                        self.timestamp(p)
                    } else {
                        None
                    })
                    .or_else(|| {
                        if set.has(Set::LINK) {
                            self.link(p)
                        } else {
                            None
                        }
                    })
                }
            }
            b'\\' => match next {
                Some('\\') => {
                    let q = self.skip_blank(p + 2);
                    if self.eol(q) && fits(q - p) && set.has(Set::LINE_BREAK) {
                        self.line_break(p)
                    } else {
                        None
                    }
                }
                Some(c)
                    if c.is_ascii_alphabetic()
                        || c == '['
                        || c == '('
                        || (c == '_' && self.byte(p + 2) == Some(b' ')) =>
                {
                    if !fits(if c == '_' { 3 } else { 2 }) {
                        return None;
                    }
                    (if set.has(Set::ENTITY) {
                        self.entity(p)
                    } else {
                        None
                    })
                    .or_else(|| {
                        if set.has(Set::LATEX_FRAGMENT) {
                            self.latex_fragment(p)
                        } else {
                            None
                        }
                    })
                }
                _ => None,
            },
            b'[' => self.try_bracket(p, set, &fits),
            _ if b.is_ascii_alphabetic() => {
                let rest = self.rest(p);
                if starts_with_ci(rest, "call_") {
                    if fits(5) && set.has(Set::INLINE_BABEL_CALL) {
                        self.inline_babel_call(p)
                    } else {
                        None
                    }
                } else if starts_with_ci(rest, "src_") {
                    if fits(4) && set.has(Set::INLINE_SRC_BLOCK) {
                        self.inline_src_block(p)
                    } else {
                        None
                    }
                } else if set.has(Set::LINK) && self.word_start(p) {
                    match link::type_prefix(rest, self.p.settings) {
                        Some(t) if rest[t.len()..].starts_with(':') && fits(t.len() + 1) => {
                            self.link(p)
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Dispatch for objects starting with `[`.
    fn try_bracket(&self, p: usize, set: Set, fits: &dyn Fn(usize) -> bool) -> Option<Obj<'a>> {
        let rest = self.rest(p + 1);
        let rb = rest.as_bytes();
        let first = *rb.first()?;
        // Length of the `org-element--object-regexp` match.
        let len = if first == b'[' {
            2
        } else if starts_with_ci(rest, "cite") && matches!(rb.get(4), Some(b':' | b'/')) {
            6
        } else if starts_with_ci(rest, "fn:") {
            4
        } else if first.is_ascii_digit() {
            2
        } else if rest.starts_with("%]") {
            3
        } else if first == b'/' {
            let digits = rb[1..].iter().take_while(|b| b.is_ascii_digit()).count();
            if rb.get(1 + digits) == Some(&b']') {
                3 + digits
            } else {
                return None;
            }
        } else {
            return None;
        };
        if !fits(len) {
            return None;
        }
        let ts_or_cookie = || {
            (if set.has(Set::TIMESTAMP) {
                self.timestamp(p)
            } else {
                None
            })
            .or_else(|| {
                if set.has(Set::STATISTICS_COOKIE) {
                    self.statistics_cookie(p)
                } else {
                    None
                }
            })
        };
        match first {
            b'[' if set.has(Set::LINK) => self.link(p),
            b'f' if set.has(Set::FOOTNOTE_REFERENCE) => self.footnote_reference(p),
            b'c' if set.has(Set::CITATION) => self.citation(p),
            b'%' | b'/' if set.has(Set::STATISTICS_COOKIE) => self.statistics_cookie(p),
            _ => ts_or_cookie(),
        }
    }
}
