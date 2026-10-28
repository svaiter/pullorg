//! Links: radio, bracket, plain and angle links (`org-element-link-parser`
//! and the `org-link-*-re` regexps of `ol.el`).

use std::ops::Range;

use super::chars::{is_punct, is_ws, starts_with_ci};
use super::{Kind, Obj, Restriction, Scanner};
use crate::event::{Link, LinkFormat, Tag};
use crate::strings::CowStr;

/// `(org-link-types)` in a stock Emacs (`emacs -Q -l org`, Org 9.8.3).
pub(crate) const LINK_TYPES: &[&str] = &[
    "id",
    "eww",
    "rmail",
    "mhe",
    "irc",
    "info",
    "gnus",
    "docview",
    "bibtex",
    "bbdb",
    "w3m",
    "doi",
    "file+sys",
    "file+emacs",
    "shell",
    "news",
    "mailto",
    "https",
    "http",
    "ftp",
    "shortdoc",
    "help",
    "file",
    "elisp",
];

/// The longest link type that is a (case-insensitive) prefix of `s`, as a
/// slice of `s`.
pub(super) fn type_prefix(s: &str) -> Option<&str> {
    LINK_TYPES
        .iter()
        .filter(|t| starts_with_ci(s, t))
        .map(|t| t.len())
        .max()
        .map(|n| &s[..n])
}

/// Sub-slice of a `CowStr`, borrowing when possible.
fn sub<'a>(s: &CowStr<'a>, r: Range<usize>) -> CowStr<'a> {
    match s {
        CowStr::Borrowed(b) => CowStr::Borrowed(&b[r]),
        CowStr::Owned(o) => CowStr::Owned(o[r].to_string()),
    }
}

/// Replace `[ \t]*\n[ \t]*` with `rep`.
fn replace_newlines<'a>(s: &'a str, rep: &str) -> CowStr<'a> {
    if !s.contains('\n') {
        return CowStr::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.split('\n').enumerate() {
        let mut line = line;
        if i > 0 {
            out.truncate(out.trim_end_matches([' ', '\t']).len());
            out.push_str(rep);
            line = line.trim_start_matches([' ', '\t']);
        }
        out.push_str(line);
    }
    CowStr::Owned(out)
}

/// `org-link-unescape`: halve backslash runs followed by a bracket or the
/// end of the string.
fn unescape(s: CowStr<'_>) -> CowStr<'_> {
    if !s.contains('\\') {
        return s;
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut seg = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            let start = i;
            while i < b.len() && b[i] == b'\\' {
                i += 1;
            }
            if i == b.len() || b[i] == b'[' || b[i] == b']' {
                out.push_str(&s[seg..start]);
                out.extend(std::iter::repeat('\\').take((i - start) / 2));
                seg = i;
            }
        } else {
            i += 1;
        }
    }
    out.push_str(&s[seg..]);
    CowStr::Owned(out)
}

/// `url-hexify-string`.
fn hexify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `org-link-expand-abbrev`.
fn expand_abbrev<'a>(link: CowStr<'a>, abbrevs: &[(CowStr<'a>, CowStr<'a>)]) -> CowStr<'a> {
    if abbrevs.is_empty() {
        return link;
    }
    let (key, tag) = match link.find(':') {
        None => (&link[..], None),
        Some(i) => {
            let rest = &link[i + 1..];
            (&link[..i], Some(rest.strip_prefix(':').unwrap_or(rest)))
        }
    };
    let Some((_, rpl)) = abbrevs.iter().find(|(k, _)| k == key) else {
        return link;
    };
    let tag = tag.unwrap_or("");
    let expanded = if rpl.contains("%(") && rpl[rpl.find("%(").unwrap()..].contains(')') {
        // A function call: Emacs only evaluates it when marked safe; we
        // never evaluate anything.
        return link;
    } else if let Some(i) = rpl.find("%s") {
        format!("{}{}{}", &rpl[..i], tag, &rpl[i + 2..])
    } else if let Some(i) = rpl.find("%h") {
        format!("{}{}{}", &rpl[..i], hexify(tag), &rpl[i + 2..])
    } else {
        format!("{rpl}{tag}")
    };
    CowStr::Owned(expanded)
}

/// `file-name-absolute-p` (Unix flavour; `~user` is not recognised).
fn is_absolute(s: &str) -> bool {
    s.starts_with('/') || s == "~" || s.starts_with("~/")
}

/// Characters allowed in a plain link path outside parentheses
/// (`[^][ \t\n()<>]`).
fn non_space_bracket(c: char) -> bool {
    !matches!(c, '[' | ']' | ' ' | '\t' | '\n' | '(' | ')' | '<' | '>')
}

impl<'p, 'a, 's> Scanner<'p, 'a, 's> {
    /// `org-element-link-parser` at `p`.
    pub(super) fn link(&self, p: usize) -> Option<Obj<'a>> {
        // Radio link.
        if !self.p.radios.is_empty() {
            let q = if self.bol(p) {
                p
            } else {
                p - self.prev(p).map_or(1, char::len_utf8)
            };
            if let Some(m) = self.radio_at(q) {
                let (ts, te) = m.target;
                // Emacs could build an object starting after its contents
                // here (and loop forever); refuse such matches.
                if ts >= p && te > p {
                    let path = self.borrow(ts..te);
                    let link = Link {
                        link_type: CowStr::Borrowed("radio"),
                        path: path.clone(),
                        raw_link: path,
                        format: LinkFormat::Plain,
                        search_option: None,
                        application: None,
                    };
                    return Some(self.obj(
                        p,
                        te,
                        Kind::Container {
                            tag: Tag::Link(link),
                            contents: Some(ts..te),
                            restriction: Restriction::LinkDescription,
                        },
                    ));
                }
            }
        }
        if let Some(o) = self.bracket_link(p) {
            return Some(o);
        }
        if let Some(o) = self.plain_link(p) {
            return Some(o);
        }
        self.angle_link(p)
    }

    /// Possible ends of the URI part of a bracket link starting at `s`,
    /// greatest first (approximating the backtracking order of
    /// `org-link-bracket-re`).
    fn bracket_uri_ends(&self, s: usize) -> Vec<usize> {
        let b = self.bytes();
        let end = self.end;
        let mut reach = vec![true];
        let mut ends = Vec::new();
        let mut i = 0;
        let mark = |reach: &mut Vec<bool>, k: usize| {
            if reach.len() <= k {
                reach.resize(k + 1, false);
            }
            reach[k] = true;
        };
        while i < reach.len() {
            if !reach[i] {
                i += 1;
                continue;
            }
            let at = s + i;
            if i > 0 {
                ends.push(at);
            }
            if at >= end {
                break;
            }
            match b[at] {
                b'[' | b']' => {}
                b'\\' => {
                    let mut m = 0;
                    while at + m < end && b[at + m] == b'\\' {
                        m += 1;
                    }
                    let after = at + m;
                    let bracket_after = after < end && matches!(b[after], b'[' | b']');
                    // \ (\\)* [][]
                    if m % 2 == 1 && bracket_after {
                        mark(&mut reach, i + m + 1);
                    }
                    // \+ [^][]
                    for j in 1..=m {
                        let c = at + j;
                        if c < end {
                            if j < m {
                                mark(&mut reach, i + j + 1);
                            } else if !bracket_after {
                                let len = self.ch(c).map_or(1, char::len_utf8);
                                mark(&mut reach, i + j + len);
                            }
                        }
                    }
                }
                _ => {
                    let len = self.ch(at).map_or(1, char::len_utf8);
                    mark(&mut reach, i + len);
                }
            }
            i += 1;
        }
        ends.reverse();
        ends
    }

    /// `org-link-bracket-re`.
    fn bracket_link(&self, p: usize) -> Option<Obj<'a>> {
        if !self.starts_with(p, "[[") {
            return None;
        }
        let s = p + 2;
        let mut found = None;
        for e in self.bracket_uri_ends(s) {
            if self.byte(e) != Some(b']') {
                continue;
            }
            match self.byte(e + 1) {
                Some(b'[') => {
                    // At least one character of description.
                    let d = e + 2;
                    let first = self.ch(d).map_or(1, char::len_utf8);
                    if let Some(q) = self.find(d + first, "]]") {
                        found = Some((s..e, Some(e + 2..q), q + 2));
                        break;
                    }
                }
                Some(b']') => {
                    found = Some((s..e, None, e + 2));
                    break;
                }
                _ => {}
            }
        }
        let (uri, desc, end) = found?;
        let source = self.slice(uri.clone());
        let raw = expand_abbrev(
            unescape(replace_newlines(source, " ")),
            &self.p.settings.link_abbreviations,
        );
        let (link_type, path): (CowStr<'a>, CowStr<'a>) =
            if is_absolute(&raw) || raw.starts_with("./") || raw.starts_with("../") {
                (CowStr::Borrowed("file"), raw)
            } else if let Some(t) = type_prefix(&raw).filter(|t| raw[t.len()..].starts_with(':')) {
                let n = t.len();
                (sub(&raw, 0..n), sub(&raw, n + 1..raw.len()))
            } else if raw.starts_with('(') && raw.ends_with(')') && raw.len() >= 2 {
                (CowStr::Borrowed("coderef"), sub(&raw, 1..raw.len() - 1))
            } else if raw.starts_with('#') {
                (CowStr::Borrowed("custom-id"), sub(&raw, 1..raw.len()))
            } else {
                (CowStr::Borrowed("fuzzy"), raw)
            };
        let link = self.finish_link(
            link_type,
            path,
            CowStr::Borrowed(source),
            LinkFormat::Bracket,
        );
        Some(self.obj(
            p,
            end,
            Kind::Container {
                tag: Tag::Link(link),
                contents: desc,
                restriction: Restriction::LinkDescription,
            },
        ))
    }

    /// `org-link-plain-re`.
    fn plain_link(&self, p: usize) -> Option<Obj<'a>> {
        if !self.word_start(p) {
            return None;
        }
        let rest = self.rest(p);
        let t = type_prefix(rest)?;
        if !rest[t.len()..].starts_with(':') {
            return None;
        }
        let ps = p + t.len() + 1;
        // Units: single characters or parenthesised groups, with whether
        // they may end the link.
        let mut units: Vec<(usize, bool)> = Vec::new();
        let mut q = ps;
        while let Some(c) = self.ch(q) {
            if non_space_bracket(c) {
                q += c.len_utf8();
                let final_ok = c == '-' || c == '/' || (!is_punct(c) && !is_ws(c) && c != '\n');
                units.push((q, final_ok));
            } else if matches!(c, '<' | '(' | '[') {
                match self.paren_group(q) {
                    Some(e) => {
                        q = e;
                        units.push((q, true));
                    }
                    None => break,
                }
            } else {
                break;
            }
        }
        let pe = (1..units.len())
            .rev()
            .find(|&i| units[i].1)
            .map(|i| units[i].0)?;
        let link = self.finish_link(
            self.borrow(p..p + t.len()),
            self.borrow(ps..pe),
            self.borrow(p..pe),
            LinkFormat::Plain,
        );
        Some(self.obj(
            p,
            pe,
            Kind::Container {
                tag: Tag::Link(link),
                contents: None,
                restriction: Restriction::LinkDescription,
            },
        ))
    }

    /// A parenthesised group in a plain link: `[<([]`, then non-bracket
    /// characters or one level of nested group, then `[])>]`.
    fn paren_group(&self, q: usize) -> Option<usize> {
        let mut i = q + 1;
        loop {
            let c = self.ch(i)?;
            if non_space_bracket(c) {
                i += c.len_utf8();
            } else if matches!(c, '<' | '(' | '[') {
                let mut j = i + 1;
                while let Some(d) = self.ch(j).filter(|&d| non_space_bracket(d)) {
                    j += d.len_utf8();
                }
                if matches!(self.byte(j), Some(b']' | b')' | b'>')) {
                    i = j + 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if matches!(self.byte(i), Some(b']' | b')' | b'>')) {
            Some(i + 1)
        } else {
            None
        }
    }

    /// `org-link-angle-re`.
    fn angle_link(&self, p: usize) -> Option<Obj<'a>> {
        if self.byte(p) != Some(b'<') {
            return None;
        }
        let rest = self.rest(p + 1);
        let t = type_prefix(rest)?;
        if !rest[t.len()..].starts_with(':') {
            return None;
        }
        let ts = p + 1;
        let ps = ts + t.len() + 1;
        let b = self.bytes();
        let mut q = ps;
        loop {
            while q < self.end && !matches!(b[q], b'>' | b'\n') {
                q += 1;
            }
            if q < self.end && b[q] == b'\n' {
                let mut r = q + 1;
                while r < self.end && matches!(b[r], b' ' | b'\t') {
                    r += 1;
                }
                if r < self.end && !matches!(b[r], b'>' | b' ' | b'\t' | b'\n') {
                    q = r;
                    continue;
                }
            }
            break;
        }
        if self.byte(q) != Some(b'>') {
            return None;
        }
        let path = replace_newlines(self.slice(ps..q), "");
        let link = self.finish_link(
            self.borrow(ts..ts + t.len()),
            path,
            self.borrow(ts..q),
            LinkFormat::Angle,
        );
        Some(self.obj(
            p,
            q + 1,
            Kind::Container {
                tag: Tag::Link(link),
                contents: None,
                restriction: Restriction::LinkDescription,
            },
        ))
    }

    /// Common post-processing: `file` links get their application and
    /// search option.
    fn finish_link(
        &self,
        link_type: CowStr<'a>,
        path: CowStr<'a>,
        raw_link: CowStr<'a>,
        format: LinkFormat,
    ) -> Link<'a> {
        let mut link = Link {
            link_type,
            path,
            raw_link,
            format,
            search_option: None,
            application: None,
        };
        let ty: &str = &link.link_type;
        let is_file = ty.eq_ignore_ascii_case("file")
            || (ty.len() > 5 && starts_with_ci(ty, "file+") && !ty[5..].contains('\n'));
        if is_file {
            if ty.len() > 5 {
                link.application = Some(sub(&link.link_type, 5..ty.len()));
            }
            link.link_type = CowStr::Borrowed("file");
            if let Some(i) = link
                .path
                .find("::")
                .filter(|i| !link.path[i + 2..].contains('\n'))
            {
                link.search_option = Some(sub(&link.path, i + 2..link.path.len()));
                link.path = sub(&link.path, 0..i);
            }
            link.path = normalize_slashes(std::mem::take(&mut link.path));
        }
        link
    }
}

/// `(replace-regexp-in-string "\\`///*\\(.:\\)?/" "\\1/" path)`.
fn normalize_slashes(path: CowStr<'_>) -> CowStr<'_> {
    let n = path.bytes().take_while(|&b| b == b'/').count();
    if n < 2 {
        return path;
    }
    for k in (0..=n - 2).rev() {
        let pos = 2 + k;
        if let Some(c) = path[pos..].chars().next() {
            let after = pos + c.len_utf8();
            if c != '\n' && path[after..].starts_with(":/") {
                return CowStr::Owned(format!("{}:/{}", c, &path[after + 2..]));
            }
            if c == '/' {
                return sub(&path, pos..path.len());
            }
        }
    }
    path
}
