//! Parsers for most object types (everything but links and timestamps).

use std::ops::Range;

use super::chars::{is_alnum, is_alpha, is_word, is_ws, latex_dollar_post};
use super::{Kind, Obj, Restriction, Scanner};
use crate::entities;
use crate::event::{Entity, Event, Tag};
use crate::options::SubSuperscripts;
use crate::strings::CowStr;

/// `org-trim`: remove leading and trailing `[ \t\n\r]`.
fn org_trim(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

/// `org-string-nw-p`: whether `s` contains a non-blank character.
fn non_blank(s: &str) -> bool {
    s.chars().any(|c| !matches!(c, ' ' | '\r' | '\t' | '\n'))
}

/// Header arguments of inline babel calls and source blocks: blank values
/// are dropped, others are trimmed and `\n[ \t]*` is replaced with a space.
fn header<'a>(s: &'a str) -> Option<CowStr<'a>> {
    if !non_blank(s) {
        return None;
    }
    let t = org_trim(s);
    if !t.contains('\n') {
        return Some(CowStr::Borrowed(t));
    }
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            out.push(' ');
            while matches!(chars.peek(), Some(' ' | '\t')) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    Some(CowStr::Owned(out))
}

/// Depth of a brace group's contents, when all its brace groups have the same
/// depth: this is what the regexp built by `org-create-multibrace-regexp`
/// accepts. `None` when nesting is irregular.
fn uniform_depth(s: &[u8], o: u8, c: u8) -> Option<usize> {
    let mut i = 0;
    let mut depth: Option<usize> = None;
    while i < s.len() {
        if s[i] == o {
            // Find the matching closing bracket.
            let mut d = 0;
            let mut j = i;
            loop {
                if s[j] == o {
                    d += 1;
                } else if s[j] == c {
                    d -= 1;
                    if d == 0 {
                        break;
                    }
                }
                j += 1;
                if j >= s.len() {
                    return None;
                }
            }
            let inner = uniform_depth(&s[i + 1..j], o, c)?;
            match depth {
                None => depth = Some(inner),
                Some(x) if x == inner => {}
                _ => return None,
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    Some(depth.map_or(0, |d| d + 1))
}

/// `org-match-sexp-depth`.
const SEXP_DEPTH: usize = 3;

impl<'p, 'a, 's> Scanner<'p, 'a, 's> {
    // ---- Emphasis -----------------------------------------------------

    /// `org-element--parse-generic-emphasis`.
    pub(super) fn emphasis(&self, p: usize, mark: u8) -> Option<Obj<'a>> {
        let markc = mark as char;
        let nonspace = |i: usize| self.ch(i).is_some_and(|c| !is_ws(c));
        let opening = if self.bol(p) {
            nonspace(p + 1)
        } else {
            let prev = self.prev(p)?;
            let q = p - prev.len_utf8();
            (self.bol(q) && prev == markc)
                || ((is_ws(prev) || matches!(prev, '-' | '(' | '\'' | '"' | '{'))
                    && nonspace(p + 1))
        };
        if !opening {
            return None;
        }
        // Closing: (not space) MARK (or post-char line-end).
        let slot = &self.cache.emphasis[match mark {
            b'*' => 0,
            b'/' => 1,
            b'_' => 2,
            b'+' => 3,
            b'~' => 4,
            _ => 5,
        }];
        let from = p + 1;
        if let Some((f, res)) = slot.get() {
            if f <= from {
                match res {
                    None => return None,
                    Some((start, m)) if start >= from => return self.emphasis_object(p, mark, m),
                    _ => {}
                }
            }
        }
        let mut s = from;
        let mut closing = None;
        while let Some(c) = self.ch(s) {
            let m = s + c.len_utf8();
            if !is_ws(c) && self.byte(m) == Some(mark) {
                let a = m + 1;
                let ok = self.eol(a)
                    || self.ch(a).is_some_and(|a| {
                        is_ws(a)
                            || matches!(
                                a,
                                '-' | '.'
                                    | ','
                                    | ';'
                                    | ':'
                                    | '!'
                                    | '?'
                                    | '\''
                                    | '"'
                                    | ')'
                                    | '}'
                                    | '\\'
                                    | '['
                            )
                    });
                if ok {
                    closing = Some((s, m));
                    break;
                }
            }
            s = m;
        }
        slot.set(Some((from, closing)));
        let (_, m) = closing?;
        self.emphasis_object(p, mark, m)
    }

    /// Build an emphasis object from the opening marker at `p` to the closing
    /// marker at `m`.
    fn emphasis_object(&self, p: usize, mark: u8, m: usize) -> Option<Obj<'a>> {
        let contents = p + 1..m;
        let kind = match mark {
            b'~' => Kind::Leaf(Event::Code(self.borrow(contents))),
            b'=' => Kind::Leaf(Event::Verbatim(self.borrow(contents))),
            _ => Kind::Container {
                tag: match mark {
                    b'*' => Tag::Bold,
                    b'/' => Tag::Italic,
                    b'_' => Tag::Underline,
                    _ => Tag::StrikeThrough,
                },
                contents: Some(contents),
                restriction: Restriction::Markup,
            },
        };
        Some(self.obj(p, m + 1, kind))
    }

    // ---- Subscript and superscript --------------------------------------

    /// `org-element-subscript-parser` / `org-element-superscript-parser`,
    /// using `org-match-substring-regexp`. `p` is on the `_` or `^`.
    pub(super) fn subsup(&self, p: usize, sup: bool) -> Option<Obj<'a>> {
        // At the beginning of a line, the character before the marker is
        // missing (subscript) or the marker itself would have to be followed
        // by another marker (superscript): no match either way.
        if self.bol(p) || self.prev(p).map_or(true, is_ws) {
            return None;
        }
        let braces_only = self.p.settings.sub_superscripts == SubSuperscripts::Braces;
        let b = p + 1;
        let (end, contents, use_brackets) = match self.byte(b)? {
            b'{' => {
                let close = self.balanced(b, b'{', b'}')?;
                let depth = uniform_depth(&self.bytes()[b + 1..close - 1], b'{', b'}')?;
                if depth >= SEXP_DEPTH {
                    return None;
                }
                (close, b + 1..close - 1, true)
            }
            _ if braces_only => return None,
            b'(' => {
                let close = self.balanced(b, b'(', b')')?;
                let depth = uniform_depth(&self.bytes()[b + 1..close - 1], b'(', b')')?;
                if depth >= SEXP_DEPTH {
                    return None;
                }
                (close, b..close, false)
            }
            b'*' => (b + 1, b..b + 1, false),
            _ => {
                let mut q = b;
                if matches!(self.byte(q), Some(b'+' | b'-')) {
                    q += 1;
                }
                let mut last_alnum = None;
                while let Some(c) = self.ch(q) {
                    if is_alnum(c) {
                        q += c.len_utf8();
                        last_alnum = Some(q);
                    } else if matches!(c, '.' | ',' | '\\') {
                        q += 1;
                    } else {
                        break;
                    }
                }
                let e = last_alnum?;
                (e, b..e, false)
            }
        };
        let tag = if sup {
            Tag::Superscript { use_brackets }
        } else {
            Tag::Subscript { use_brackets }
        };
        Some(self.obj(
            p,
            end,
            Kind::Container {
                tag,
                contents: Some(contents),
                restriction: Restriction::SubSuperscript,
            },
        ))
    }

    // ---- Entities, LaTeX fragments and line breaks --------------------

    /// `org-element-entity-parser`. `p` is on the backslash.
    pub(super) fn entity(&self, p: usize) -> Option<Obj<'a>> {
        let s = p + 1;
        let rest = self.rest(s);
        let (name_end, brackets) = if let Some(spaces) = rest.strip_prefix('_') {
            let n = spaces.bytes().take_while(|&b| b == b' ').count();
            if n == 0 {
                return None;
            }
            (s + 1 + n, false)
        } else {
            let rb = rest.as_bytes();
            let ci = |lit: &str| super::chars::starts_with_ci(rest, lit);
            let in_ = |i: usize, set: &[u8]| {
                rb.get(i)
                    .is_some_and(|b| set.contains(&b.to_ascii_lowercase()))
            };
            let letters = rb.iter().take_while(|b| b.is_ascii_alphabetic()).count();
            // The alternatives of the name group, in order.
            let candidates = [
                if ci("there4") { Some(6) } else { None },
                if ci("sup") && in_(3, b"123") {
                    Some(4)
                } else {
                    None
                },
                if ci("frac") && in_(4, b"13") && in_(5, b"24") {
                    Some(6)
                } else {
                    None
                },
                if letters > 0 { Some(letters) } else { None },
            ];
            let mut found = None;
            for len in candidates.into_iter().flatten() {
                let e = s + len;
                if self.eol(e) {
                    found = Some((e, false));
                } else if self.starts_with(e, "{}") {
                    found = Some((e, true));
                } else if self.ch(e).is_some_and(|c| !is_alpha(c)) {
                    found = Some((e, false));
                }
                if found.is_some() {
                    break;
                }
            }
            found?
        };
        let name = self.slice(s..name_end);
        let def = entities::lookup(name)?;
        let end = if brackets { name_end + 2 } else { name_end };
        Some(self.obj(
            p,
            end,
            Kind::Leaf(Event::Entity(Entity {
                name: def.name,
                latex: def.latex,
                latex_math: def.latex_math,
                html: def.html,
                ascii: def.ascii,
                latin1: def.latin1,
                utf8: def.utf8,
                use_brackets: brackets,
                _marker: std::marker::PhantomData,
            })),
        ))
    }

    /// `org-element-latex-fragment-parser`.
    pub(super) fn latex_fragment(&self, p: usize) -> Option<Obj<'a>> {
        let after = if self.byte(p) != Some(b'$') {
            match self.byte(p + 1)? {
                b'(' => self.find(p, "\\)")? + 2,
                b'[' => self.find(p, "\\]")? + 2,
                _ => {
                    // \\[a-zA-Z]+\*?\(\(\[[^][\n{}]*\]\)\|\({[^{}\n]*}\)\)*
                    let b = self.bytes();
                    let mut q = p + 1;
                    while self.byte(q).is_some_and(|c| c.is_ascii_alphabetic()) {
                        q += 1;
                    }
                    if q == p + 1 {
                        return None;
                    }
                    if self.byte(q) == Some(b'*') {
                        q += 1;
                    }
                    loop {
                        let (o, c, bad): (u8, u8, &[u8]) = match self.byte(q) {
                            Some(b'[') => (b'[', b']', b"[]\n{}"),
                            Some(b'{') => (b'{', b'}', b"{}\n"),
                            _ => break,
                        };
                        let _ = o;
                        let mut r = q + 1;
                        while r < self.end && !bad.contains(&b[r]) {
                            r += 1;
                        }
                        if r < self.end && b[r] == c {
                            q = r + 1;
                        } else {
                            break;
                        }
                    }
                    q
                }
            }
        } else if self.byte(p + 1) == Some(b'$') {
            self.find(p + 2, "$$")? + 2
        } else {
            if self.prev(p) == Some('$') {
                return None;
            }
            if matches!(
                self.byte(p + 1),
                Some(b' ' | b'\t' | b'\n' | b',' | b'.' | b';')
            ) {
                return None;
            }
            let q = self.find(p + 1, "$")?;
            if matches!(self.prev(q), Some(' ' | '\t' | '\n' | ',' | '.')) {
                return None;
            }
            if !(self.eol(q + 1) || self.ch(q + 1).is_some_and(latex_dollar_post)) {
                return None;
            }
            q + 1
        };
        Some(self.obj(
            p,
            after,
            Kind::Leaf(Event::LatexFragment(self.borrow(p..after))),
        ))
    }

    /// `org-element-line-break-parser`. The range includes the trailing
    /// blanks and the newline.
    pub(super) fn line_break(&self, p: usize) -> Option<Obj<'a>> {
        if !self.starts_with(p, "\\\\") || self.prev(p) == Some('\\') {
            return None;
        }
        let q = self.skip_blank(p + 2);
        if !self.eol(q) {
            return None;
        }
        // Emacs' `:end` includes the newline. The content range handed to us
        // excludes the trailing newline of the container, so peek past it to
        // keep the range faithful while never scanning beyond the range.
        let newline_follows = self.bytes().get(q) == Some(&b'\n');
        let end = if newline_follows { q + 1 } else { q };
        Some(Obj {
            begin: p,
            end,
            after: end.min(self.end),
            kind: Kind::Leaf(Event::LineBreak),
        })
    }

    // ---- Simple leaves --------------------------------------------------

    /// `org-element-export-snippet-parser`.
    pub(super) fn export_snippet(&self, p: usize) -> Option<Obj<'a>> {
        let rest = self.rest(p + 2);
        let n = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-')
            .count();
        if n == 0 || rest.as_bytes().get(n) != Some(&b':') {
            return None;
        }
        let vb = p + 2 + n + 1;
        let ve = self.find(vb, "@@")?;
        Some(self.obj(
            p,
            ve + 2,
            Kind::Leaf(Event::ExportSnippet {
                backend: self.borrow(p + 2..p + 2 + n),
                value: self.borrow(vb..ve),
            }),
        ))
    }

    /// `org-element-footnote-reference-parser`, with `org-footnote-re`.
    pub(super) fn footnote_reference(&self, p: usize) -> Option<Obj<'a>> {
        let rest = self.rest(p);
        if !super::chars::starts_with_ci(rest, "[fn:") {
            return None;
        }
        let lb = p + 4;
        let mut le = lb;
        while let Some(c) = self.ch(le).filter(|&c| c == '-' || c == '_' || is_word(c)) {
            le += c.len_utf8();
        }
        let inline = match self.byte(le) {
            Some(b':') => true,
            Some(b']') if le > lb => false,
            _ => return None,
        };
        let closing = self.balanced(p, b'[', b']')?;
        let label = (le > lb).then(|| self.borrow(lb..le));
        let kind = if inline {
            Kind::Container {
                tag: Tag::FootnoteReference { label },
                contents: Some(le + 1..closing - 1),
                restriction: Restriction::FootnoteReference,
            }
        } else {
            Kind::Leaf(Event::FootnoteReference(label?))
        };
        Some(self.obj(p, closing, kind))
    }

    /// Contents of the paired brackets at `p` (`org-element--parse-paired-brackets`).
    fn paired(&self, p: usize, o: u8, c: u8) -> Option<(Range<usize>, usize)> {
        let close = self.balanced(p, o, c)?;
        Some((p + 1..close - 1, close))
    }

    /// `[^ \t\n<excluded>]+` followed by one of `next`, after `prefix` at `p`.
    fn name_after(
        &self,
        p: usize,
        prefix: &str,
        excluded: &[u8],
        next: &[u8],
    ) -> Option<Range<usize>> {
        if !self.word_start(p) || !self.starts_with(p, prefix) {
            return None;
        }
        let nb = p + prefix.len();
        let rest = self.bytes();
        let mut ne = nb;
        while ne < self.end
            && !matches!(rest[ne], b' ' | b'\t' | b'\n')
            && !excluded.contains(&rest[ne])
        {
            ne += 1;
        }
        if ne == nb || !next.contains(&self.byte(ne)?) {
            return None;
        }
        Some(nb..ne)
    }

    /// `org-element-inline-babel-call-parser`.
    pub(super) fn inline_babel_call(&self, p: usize) -> Option<Obj<'a>> {
        let name = self.name_after(p, "call_", b"[(", b"([")?;
        let mut q = name.end;
        let inside = self.paired(q, b'[', b']').map(|(r, e)| {
            q = e;
            r
        });
        let (args, e) = self.paired(q, b'(', b')')?;
        q = e;
        let end_header = self.paired(q, b'[', b']').map(|(r, e)| {
            q = e;
            r
        });
        Some(self.obj(
            p,
            q,
            Kind::Leaf(Event::InlineBabelCall {
                call: self.borrow(name),
                inside_header: inside.and_then(|r| header(self.slice(r))),
                arguments: non_blank(self.slice(args.clone())).then(|| self.borrow(args)),
                end_header: end_header.and_then(|r| header(self.slice(r))),
            }),
        ))
    }

    /// `org-element-inline-src-block-parser`.
    pub(super) fn inline_src_block(&self, p: usize) -> Option<Obj<'a>> {
        let lang = self.name_after(p, "src_", b"[{", b"{[")?;
        let mut q = lang.end;
        let params = self.paired(q, b'[', b']').map(|(r, e)| {
            q = e;
            r
        });
        let (value, e) = self.paired(q, b'{', b'}')?;
        Some(self.obj(
            p,
            e,
            Kind::Leaf(Event::InlineSrcBlock {
                language: self.borrow(lang),
                parameters: params.and_then(|r| header(self.slice(r))),
                value: self.borrow(value),
            }),
        ))
    }

    /// `org-element-macro-parser`.
    pub(super) fn macro_(&self, p: usize) -> Option<Obj<'a>> {
        let kb = p + 3;
        if !self.byte(kb)?.is_ascii_alphabetic() {
            return None;
        }
        let mut ke = kb + 1;
        while self
            .byte(ke)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            ke += 1;
        }
        let (args, end) = if self.byte(ke) == Some(b'(') {
            let close = self.find(ke + 1, ")}}}")?;
            (Some(ke + 1..close), close + 4)
        } else if self.starts_with(ke, "}}}") {
            (None, ke + 3)
        } else {
            return None;
        };
        let key = self.slice(kb..ke);
        let key = if key.bytes().any(|b| b.is_ascii_uppercase()) {
            CowStr::Owned(key.to_ascii_lowercase())
        } else {
            CowStr::Borrowed(key)
        };
        let args = args.map_or_else(Vec::new, |r| macro_arguments(self.slice(r)));
        Some(self.obj(p, end, Kind::Leaf(Event::Macro { key, args })))
    }

    /// Inner part of `org-target-regexp` at `q`, followed by `close`.
    fn target_inner(&self, q: usize, close: &str) -> Option<usize> {
        let b = self.bytes();
        let mut e = q;
        while e < self.end && !matches!(b[e], b'<' | b'>' | b'\n' | b'\r') {
            e += 1;
        }
        if e == q
            || matches!(b[q], b' ' | b'\t')
            || matches!(b[e - 1], b' ' | b'\t')
            || !self.starts_with(e, close)
        {
            return None;
        }
        Some(e)
    }

    /// `org-element-radio-target-parser`.
    pub(super) fn radio_target(&self, p: usize) -> Option<Obj<'a>> {
        if !self.starts_with(p, "<<<") {
            return None;
        }
        let e = self.target_inner(p + 3, ">>>")?;
        Some(self.obj(
            p,
            e + 3,
            Kind::Container {
                tag: Tag::RadioTarget,
                contents: Some(p + 3..e),
                restriction: Restriction::RadioTarget,
            },
        ))
    }

    /// `org-element-target-parser`.
    pub(super) fn target(&self, p: usize) -> Option<Obj<'a>> {
        if !self.starts_with(p, "<<") {
            return None;
        }
        let e = self.target_inner(p + 2, ">>")?;
        Some(self.obj(p, e + 2, Kind::Leaf(Event::Target(self.borrow(p + 2..e)))))
    }

    /// `org-element-statistics-cookie-parser`.
    pub(super) fn statistics_cookie(&self, p: usize) -> Option<Obj<'a>> {
        let b = self.rest(p).as_bytes();
        if b.first() != Some(&b'[') {
            return None;
        }
        let mut i = 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        match b.get(i) {
            Some(b'%') => i += 1,
            Some(b'/') => {
                i += 1;
                while b.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
            }
            _ => return None,
        }
        if b.get(i) != Some(&b']') {
            return None;
        }
        let end = p + i + 1;
        Some(self.obj(
            p,
            end,
            Kind::Leaf(Event::StatisticsCookie(self.borrow(p..end))),
        ))
    }

    // ---- Citations ------------------------------------------------------

    /// `org-element-citation-key-re` searched from `p` to `bound`: returns
    /// the range of the whole match (`@key`).
    fn citation_key(&self, p: usize, bound: usize) -> Option<Range<usize>> {
        let is_key = |c: char| is_word(c) || "-.:?!`'/*@+|(){}<>&_^$#%~".contains(c);
        let text = &self.text()[p..bound.min(self.end)];
        for (i, c) in text.char_indices() {
            if c == '@' {
                let ks = i + 1;
                let kl: usize = text[ks..]
                    .chars()
                    .take_while(|&c| is_key(c))
                    .map(char::len_utf8)
                    .sum();
                if kl > 0 {
                    return Some(p + i..p + ks + kl);
                }
            }
        }
        None
    }

    /// `org-element-citation-parser`.
    pub(super) fn citation(&self, p: usize) -> Option<Obj<'a>> {
        let rest = self.rest(p);
        if !super::chars::starts_with_ci(rest, "[cite") {
            return None;
        }
        let mut q = p + 5;
        let mut style = None;
        if self.byte(q) == Some(b'/') {
            let n: usize = self
                .rest(q + 1)
                .chars()
                .take_while(|&c| matches!(c, '/' | '_' | '-') || is_alnum(c))
                .map(char::len_utf8)
                .sum();
            if n > 0 && self.byte(q + 1 + n) == Some(b':') {
                style = Some(q + 1..q + 1 + n);
                q += 1 + n;
            }
        }
        if self.byte(q) != Some(b':') {
            return None;
        }
        q += 1;
        while matches!(self.byte(q), Some(b' ' | b'\t' | b'\n')) {
            q += 1;
        }
        let start = q;
        let closing = self.balanced(p, b'[', b']')?;
        let first_key = self.citation_key(start, closing)?;
        let first_key_end = first_key.end;
        // Common prefix.
        let (prefix, contents_begin) = match self.slice(start..first_key_end).rfind(';') {
            None => (None, start),
            Some(i) => {
                let semi = start + i;
                ((semi > start).then(|| self.borrow(start..semi)), semi + 1)
            }
        };
        // Common suffix.
        let mut end = closing - 1;
        while end > start && matches!(self.bytes()[end - 1], b' ' | b'\r' | b'\t' | b'\n') {
            end -= 1;
        }
        let end = end.max(first_key_end);
        let (suffix, contents_end) = match self.slice(first_key_end..end).rfind(';') {
            Some(i) if self.citation_key(first_key_end + i, end).is_none() => {
                let s = first_key_end + i + 1;
                ((s < end).then(|| self.borrow(s..end)), s)
            }
            _ => (None, end),
        };
        Some(self.obj(
            p,
            closing,
            Kind::Citation {
                tag: Tag::Citation {
                    style: style.map(|r| self.borrow(r)),
                    prefix,
                    suffix,
                },
                contents: contents_begin..contents_end,
            },
        ))
    }

    /// `org-element-citation-reference-parser`, at `p` inside a citation.
    pub(super) fn citation_reference(&self, p: usize) -> Option<Obj<'a>> {
        let key = self.citation_key(p, self.end)?;
        let sep = self.find(key.end, ";");
        let end = sep.map_or(self.end, |s| s + 1);
        let suffix_end = sep.unwrap_or(self.end);
        Some(Obj {
            begin: p,
            end,
            after: end,
            kind: Kind::Leaf(Event::CitationReference {
                key: self.borrow(key.start + 1..key.end),
                prefix: (p < key.start).then(|| self.borrow(p..key.start)),
                suffix: (key.end < suffix_end).then(|| self.borrow(key.end..suffix_end)),
            }),
        })
    }
}

/// Macro arguments: `org-trim`, collapse whitespace, then
/// `org-macro-extract-arguments` (split on commas not escaped by an odd
/// number of backslashes; halve backslash runs before commas).
fn macro_arguments<'a>(raw: &'a str) -> Vec<CowStr<'a>> {
    let t = org_trim(raw);
    let is_blank = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n');
    let needs_collapse = t.bytes().any(|b| matches!(b, b'\t' | b'\r' | b'\n'))
        || t.as_bytes()
            .windows(2)
            .any(|w| is_blank(w[0]) && is_blank(w[1]));
    if !needs_collapse && !t.contains('\\') {
        return t.split(',').map(CowStr::Borrowed).collect();
    }
    let mut s = String::with_capacity(t.len());
    let mut in_ws = false;
    for c in t.chars() {
        if matches!(c, ' ' | '\t' | '\r' | '\n') {
            if !in_ws {
                s.push(' ');
            }
            in_ws = true;
        } else {
            s.push(c);
            in_ws = false;
        }
    }
    let b = s.as_bytes();
    let mut args = Vec::new();
    let mut current = String::new();
    let mut seg_start = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' || b[i] == b',' {
            let run_start = i;
            while i < b.len() && b[i] == b'\\' {
                i += 1;
            }
            if i < b.len() && b[i] == b',' {
                let n = i - run_start;
                current.push_str(&s[seg_start..run_start]);
                current.extend(std::iter::repeat('\\').take(n / 2));
                if n % 2 == 0 {
                    args.push(CowStr::Owned(std::mem::take(&mut current)));
                } else {
                    current.push(',');
                }
                i += 1;
                seg_start = i;
            }
        } else {
            i += 1;
        }
    }
    current.push_str(&s[seg_start..]);
    args.push(CowStr::Owned(current));
    args
}
