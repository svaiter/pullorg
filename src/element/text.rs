//! Low level helpers working on byte offsets of the source text.
//!
//! The element parser mimics the line oriented regular expressions of
//! `org-element.el`. Instead of a regex engine, each pattern is matched by a
//! small hand written function. All offsets are byte offsets; every pattern
//! only stops on ASCII bytes (or on char boundaries obtained by decoding), so
//! slicing never splits a UTF-8 sequence.

/// Whitespace in the sense of Emacs' `\s-` (whitespace syntax class) as far
/// as Org documents are concerned.
pub(crate) fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// `[:word:]` / `\w`: letters and digits, including non-ASCII ones.
pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// Case insensitive string equality (Emacs `case-fold-search`).
pub(crate) fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || (a.len() == b.len() && a.to_lowercase() == b.to_lowercase())
}

/// The source text with line utilities.
#[derive(Clone, Copy)]
pub(crate) struct Src<'a> {
    pub text: &'a str,
    pub b: &'a [u8],
}

impl<'a> Src<'a> {
    pub fn new(text: &'a str) -> Self {
        Src {
            text,
            b: text.as_bytes(),
        }
    }

    pub fn len(&self) -> usize {
        self.b.len()
    }

    /// Byte at `p`, or 0 past the end.
    pub fn at(&self, p: usize) -> u8 {
        if p < self.b.len() {
            self.b[p]
        } else {
            0
        }
    }

    /// Char starting at byte `p`, if any.
    pub fn char_at(&self, p: usize) -> Option<char> {
        self.text.get(p..).and_then(|s| s.chars().next())
    }

    pub fn is_bol(&self, p: usize) -> bool {
        p == 0 || self.b[p - 1] == b'\n'
    }

    /// Beginning of the line containing `p`.
    pub fn bol(&self, p: usize) -> usize {
        let mut i = p.min(self.len());
        while i > 0 && self.b[i - 1] != b'\n' {
            i -= 1;
        }
        i
    }

    /// End of the line containing `p` (position of the newline, or the end
    /// of the text).
    pub fn eol(&self, p: usize) -> usize {
        let mut i = p;
        while i < self.len() && self.b[i] != b'\n' {
            i += 1;
        }
        i
    }

    /// Beginning of the next line (Emacs `line-beginning-position 2`, or
    /// `forward-line` from `p`). Returns the end of the text on the last
    /// line.
    pub fn next_line(&self, p: usize) -> usize {
        let e = self.eol(p);
        if e < self.len() {
            e + 1
        } else {
            e
        }
    }

    /// Beginning of the previous line, if any.
    pub fn prev_line(&self, p: usize) -> Option<usize> {
        let b = self.bol(p);
        if b == 0 {
            None
        } else {
            Some(self.bol(b - 1))
        }
    }

    /// Skip `[ \t]*` forward.
    pub fn skip_blank(&self, mut p: usize) -> usize {
        while p < self.len() && matches!(self.b[p], b' ' | b'\t') {
            p += 1;
        }
        p
    }

    /// Skip `[ \t]*` forward, without going past `lim`.
    pub fn skip_blank_to(&self, mut p: usize, lim: usize) -> usize {
        while p < lim && matches!(self.b[p], b' ' | b'\t') {
            p += 1;
        }
        p
    }

    /// Skip `[ \t]*` backward, not before `lo`.
    pub fn skip_blank_back(&self, mut p: usize, lo: usize) -> usize {
        while p > lo && matches!(self.b[p - 1], b' ' | b'\t') {
            p -= 1;
        }
        p
    }

    /// `skip-chars-forward " \r\t\n" lim`.
    pub fn skip_ws(&self, mut p: usize, lim: usize) -> usize {
        while p < lim && matches!(self.b[p], b' ' | b'\t' | b'\r' | b'\n') {
            p += 1;
        }
        p
    }

    /// `skip-chars-backward " \r\t\n" lo`.
    pub fn skip_ws_back(&self, mut p: usize, lo: usize) -> usize {
        while p > lo && matches!(self.b[p - 1], b' ' | b'\t' | b'\r' | b'\n') {
            p -= 1;
        }
        p
    }

    /// The idiom ending most element parsers: skip blank lines up to `lim`
    /// and return the beginning of the next non blank line (or the end of
    /// the text).
    pub fn after_blank(&self, p: usize, lim: usize) -> usize {
        let q = self.skip_ws(p, lim);
        if q >= self.len() {
            q
        } else {
            self.bol(q)
        }
    }

    /// Variant used by plain lists: `(if (= (point) limit) limit (bol))`.
    pub fn after_blank_lim(&self, p: usize, lim: usize) -> usize {
        let q = self.skip_ws(p, lim);
        if q == lim {
            lim
        } else {
            self.bol(q)
        }
    }

    /// `(progn (skip-chars-backward " \r\t\n" lo) (line-beginning-position 2))`.
    pub fn contents_end(&self, p: usize, lo: usize) -> usize {
        let q = self.skip_ws_back(p, lo);
        self.next_line(q)
    }

    /// Remove trailing blank lines from `begin..end`: the result is the
    /// beginning of the line following the last non blank character, never
    /// past `end`.
    pub fn trim_end(&self, begin: usize, end: usize) -> usize {
        let q = self.skip_ws_back(end, begin);
        if q == end {
            return end;
        }
        self.next_line(q).min(end)
    }

    /// Whether the line starting at (or containing) `p` is blank from `p` to
    /// its end (`[ \t]*$`).
    pub fn blank_from(&self, p: usize) -> bool {
        let q = self.skip_blank(p);
        q >= self.len() || self.b[q] == b'\n'
    }

    /// `[ \t]*$` ending exactly at the end of the line: everything from `p`
    /// to the end of line is blank.
    pub fn rest_blank(&self, p: usize) -> bool {
        self.blank_from(p)
    }

    /// Case insensitive (ASCII) prefix test.
    pub fn starts_ci(&self, p: usize, pat: &str) -> bool {
        let pb = pat.as_bytes();
        p + pb.len() <= self.len() && self.b[p..p + pb.len()].eq_ignore_ascii_case(pb)
    }

    /// Case sensitive prefix test.
    pub fn starts(&self, p: usize, pat: &str) -> bool {
        self.b[p.min(self.len())..].starts_with(pat.as_bytes())
    }

    /// Run of non whitespace chars (`\S-+`) starting at `p`; returns its end.
    pub fn non_space_run(&self, p: usize) -> usize {
        let mut i = p;
        while let Some(c) = self.char_at(i) {
            if is_space(c) {
                break;
            }
            i += c.len_utf8();
        }
        i
    }

    /// Iterate over line beginnings `q` with `from <= q < lim`, `from`
    /// included only if it is itself at a beginning of line, and return the
    /// first one for which `pred` holds.
    pub fn find_line(
        &self,
        from: usize,
        lim: usize,
        mut pred: impl FnMut(usize) -> bool,
    ) -> Option<usize> {
        let mut q = if self.is_bol(from) {
            from
        } else {
            let e = self.eol(from);
            if e >= self.len() {
                return None;
            }
            e + 1
        };
        loop {
            if q >= lim {
                return None;
            }
            if pred(q) {
                return Some(q);
            }
            let e = self.eol(q);
            if e >= self.len() {
                return None;
            }
            q = e + 1;
        }
    }

    /// Indentation width of the line at `p` (bol), as
    /// `org-current-text-column` computes it: the `string-width` of the
    /// leading blanks, where a tab always counts for 8 (not a tab stop).
    pub fn indent_col(&self, p: usize) -> usize {
        let mut col = 0;
        let mut i = p;
        while i < self.len() {
            match self.b[i] {
                b' ' => col += 1,
                b'\t' => col += 8,
                _ => break,
            }
            i += 1;
        }
        col
    }

    /// Slice helper.
    pub fn s(&self, r: std::ops::Range<usize>) -> &'a str {
        &self.text[r]
    }

    /// End of a run of ASCII digits starting at `p`.
    pub fn digits(&self, p: usize) -> usize {
        let mut i = p;
        while i < self.len() && self.b[i].is_ascii_digit() {
            i += 1;
        }
        i
    }

    // ------------------------------------------------------------------
    // Line patterns.

    /// `^\*+ `: returns the number of stars.
    pub fn headline_stars(&self, p: usize) -> Option<usize> {
        if !self.is_bol(p) {
            return None;
        }
        let mut i = p;
        while self.at(i) == b'*' {
            i += 1;
        }
        if i > p && self.at(i) == b' ' {
            Some(i - p)
        } else {
            None
        }
    }

    /// `org-comment-regexp`: `^[ \t]*#( |$)`.
    pub fn comment_line(&self, p: usize) -> Option<usize> {
        let i = self.skip_blank(p);
        if self.at(i) != b'#' {
            return None;
        }
        if self.at(i + 1) == b' ' {
            Some(i + 2)
        } else if i + 1 >= self.len() || self.b[i + 1] == b'\n' {
            Some(i + 1)
        } else {
            None
        }
    }

    /// `org-element-drawer-re`: `^[ \t]*:([-_\w]+):[ \t]*$`. Returns the
    /// name range.
    pub fn drawer_line(&self, p: usize) -> Option<std::ops::Range<usize>> {
        let i = self.skip_blank(p);
        if self.at(i) != b':' {
            return None;
        }
        let start = i + 1;
        let mut j = start;
        while let Some(c) = self.char_at(j) {
            if c == '-' || c == '_' || is_word(c) {
                j += c.len_utf8();
            } else {
                break;
            }
        }
        if j == start || self.at(j) != b':' {
            return None;
        }
        if self.rest_blank(j + 1) {
            Some(start..j)
        } else {
            None
        }
    }

    /// `^[ \t]*:END:[ \t]*$` (case insensitive).
    pub fn end_drawer_line(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        self.starts_ci(i, ":END:") && self.rest_blank(i + 5)
    }

    /// `^[ \t]*#\+END_<name>[ \t]*$` (case insensitive).
    pub fn block_end_line(&self, p: usize, name: &str) -> bool {
        let i = self.skip_blank(p);
        if !self.starts_ci(i, "#+END_") {
            return false;
        }
        let n = i + 6;
        let e = n + name.len();
        if e > self.len() || !self.text.is_char_boundary(e) {
            return false;
        }
        eq_ci(&self.text[n..e], name) && self.rest_blank(e)
    }

    /// Search a block end line from `from` (bol) to `lim`.
    pub fn find_block_end(&self, from: usize, lim: usize, name: &str) -> Option<usize> {
        self.find_line(from, lim, |q| self.block_end_line(q, name))
    }

    /// `org-element--latex-begin-environment`: `^[ \t]*\\begin{([A-Za-z0-9*]+)}`.
    pub fn latex_begin(&self, p: usize) -> Option<std::ops::Range<usize>> {
        let i = self.skip_blank(p);
        if !self.starts(i, "\\begin{") {
            return None;
        }
        let s = i + 7;
        let mut j = s;
        while j < self.len() && (self.b[j].is_ascii_alphanumeric() || self.b[j] == b'*') {
            j += 1;
        }
        if j > s && self.at(j) == b'}' {
            Some(s..j)
        } else {
            None
        }
    }

    /// Search `\\end{name}[ \t]*$` from `from` (not anchored at bol), whole
    /// match ending before `lim`. Returns the position after the match.
    pub fn find_latex_end(&self, from: usize, lim: usize, name: &str) -> Option<usize> {
        let pat = format!("\\end{{{}}}", name);
        let mut start = from;
        while let Some(off) = self.text.get(start..lim).and_then(|s| {
            // case-fold: the environment name is ASCII.
            find_ci(s, &pat)
        }) {
            let m = start + off;
            let after = m + pat.len();
            let e = self.skip_blank(after);
            if (e >= self.len() || self.b[e] == b'\n') && e <= lim {
                return Some(e);
            }
            start = m + 1;
        }
        None
    }

    /// `[ \t]*-----+[ \t]*$`
    pub fn hrule_line(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        let mut j = i;
        while self.at(j) == b'-' {
            j += 1;
        }
        j - i >= 5 && self.rest_blank(j)
    }

    /// `[ \t]*:( |$)`: fixed width line.
    pub fn fixed_width_line(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        self.at(i) == b':'
            && (self.at(i + 1) == b' ' || i + 1 >= self.len() || self.b[i + 1] == b'\n')
    }

    /// `^\[fn:([-_\w]+)\]`: footnote definition label range.
    pub fn footnote_def(&self, p: usize) -> Option<std::ops::Range<usize>> {
        if !self.is_bol(p) || !self.starts(p, "[fn:") {
            return None;
        }
        let s = p + 4;
        let mut j = s;
        while let Some(c) = self.char_at(j) {
            if c == '-' || c == '_' || is_word(c) {
                j += c.len_utf8();
            } else {
                break;
            }
        }
        if j > s && self.at(j) == b']' {
            Some(s..j)
        } else {
            None
        }
    }

    /// `^[ \t]*#\+BEGIN:[ \t]*\S`: dynamic block opening line (case
    /// insensitive).
    pub fn dynblock_open(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        if !self.starts_ci(i, "#+BEGIN:") {
            return false;
        }
        let j = self.skip_blank(i + 8);
        matches!(self.char_at(j), Some(c) if !is_space(c))
    }

    /// `org-element-clock-line-re` (case insensitive `CLOCK:`).
    pub fn clock_line_full(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        if !self.starts_ci(i, "CLOCK:") {
            return false;
        }
        let eol = self.eol(i);
        let rest = &self.b[i + 6..eol];
        clock_rest(rest)
    }

    /// `org-clock-line-re`: `^[ \t]*CLOCK:` (case insensitive).
    pub fn clock_line_prefix(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        self.starts_ci(i, "CLOCK:")
    }

    /// `org-element-planning-line-re` (case insensitive).
    pub fn planning_line(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        self.starts_ci(i, "CLOSED:")
            || self.starts_ci(i, "DEADLINE:")
            || self.starts_ci(i, "SCHEDULED:")
    }

    /// `org-item-re` (`org-list-allow-alphabetical` nil, both terminators):
    /// `[ \t]*([-+]|[0-9]+[.)])([ \t]+|$)` or `[ \t]+\*([ \t]+|$)`.
    pub fn item_line(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        let j = match self.at(i) {
            b'-' | b'+' => i + 1,
            b'*' if i > p => i + 1,
            c if c.is_ascii_digit() => {
                let d = self.digits(i);
                if matches!(self.at(d), b'.' | b')') {
                    d + 1
                } else {
                    return false;
                }
            }
            _ => return false,
        };
        matches!(self.at(j), b' ' | b'\t' | b'\n') || j >= self.len()
    }

    /// Org table start: `[ \t]*|`.
    pub fn table_line(&self, p: usize) -> bool {
        self.at(self.skip_blank(p)) == b'|'
    }

    /// `[ \t]*\+(-+\+)+[ \t]*$`: a `table.el` rule.
    pub fn tableel_rule(&self, p: usize) -> bool {
        let i = self.skip_blank(p);
        if self.at(i) != b'+' {
            return false;
        }
        let mut j = i + 1;
        let mut groups = 0;
        loop {
            let s = j;
            while self.at(j) == b'-' {
                j += 1;
            }
            if j > s && self.at(j) == b'+' {
                j += 1;
                groups += 1;
            } else {
                j = s;
                break;
            }
        }
        groups > 0 && self.rest_blank(j)
    }

    /// `org-property-drawer-re` at `p` (case insensitive). Returns the
    /// beginning of the `:END:` line.
    pub fn property_drawer(&self, p: usize) -> Option<usize> {
        let i = self.skip_blank(p);
        if !self.starts_ci(i, ":PROPERTIES:") || !self.rest_blank(i + 12) {
            return None;
        }
        let mut q = self.next_line(p);
        if q == self.eol(p) {
            // No newline after :PROPERTIES:.
            return None;
        }
        loop {
            if q >= self.len() {
                return None;
            }
            if self.end_drawer_line(q) {
                return Some(q);
            }
            // [ \t]*:\S-+:(?:[ \t].*)?[ \t]*\n
            if !self.property_like_line(q) {
                return None;
            }
            let e = self.eol(q);
            if e >= self.len() {
                return None;
            }
            q = e + 1;
        }
    }

    /// `[ \t]*:\S-+:(?:[ \t].*)?[ \t]*$`
    fn property_like_line(&self, q: usize) -> bool {
        let i = self.skip_blank(q);
        if self.at(i) != b':' {
            return false;
        }
        let run_end = self.non_space_run(i + 1);
        // \S-+ then ':' : need a ':' at index >= i+2 within the run, after
        // which comes either the end of the run (then whitespace / eol).
        // The char right after the closing ':' must be [ \t] or eol, which
        // only holds when the ':' ends the run.
        run_end >= i + 3 && self.b[run_end - 1] == b':'
    }
}

/// ASCII case-insensitive substring search.
pub(crate) fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() {
        return Some(0);
    }
    if h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// Inactive timestamp skeleton `\[(\d{4}-\d{2}-\d{2}(?: .*?)?)\]` followed
/// by `tail`, where the lazy `.*?` may extend as needed. `s` starts at `[`.
fn inactive_then(s: &[u8], tail: &dyn Fn(&[u8]) -> bool) -> bool {
    if s.len() < 12 || s[0] != b'[' {
        return false;
    }
    let d = &s[1..11];
    let ok = d[..4].iter().all(u8::is_ascii_digit)
        && d[4] == b'-'
        && d[5..7].iter().all(u8::is_ascii_digit)
        && d[7] == b'-'
        && d[8..10].iter().all(u8::is_ascii_digit);
    if !ok {
        return false;
    }
    // Either "]" right after the date, or " " then anything then "]".
    if s[11] == b']' && tail(&s[12..]) {
        return true;
    }
    if s[11] == b' ' {
        for k in 12..s.len() {
            if s[k] == b']' && tail(&s[k + 1..]) {
                return true;
            }
        }
    }
    false
}

/// `[ \t]+=>[ \t]+\d+:\d\d`, returns rest.
fn duration(s: &[u8]) -> Option<&[u8]> {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t') {
        i += 1;
    }
    if i == 0 || !s[i..].starts_with(b"=>") {
        return None;
    }
    i += 2;
    let j0 = i;
    while i < s.len() && matches!(s[i], b' ' | b'\t') {
        i += 1;
    }
    if i == j0 {
        return None;
    }
    let d0 = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    if i == d0
        || i + 3 > s.len()
        || s[i] != b':'
        || !s[i + 1].is_ascii_digit()
        || !s[i + 2].is_ascii_digit()
    {
        return None;
    }
    Some(&s[i + 3..])
}

fn blank(s: &[u8]) -> bool {
    s.iter().all(|&c| c == b' ' || c == b'\t')
}

/// Rest of a clock line after `CLOCK:`.
fn clock_rest(s: &[u8]) -> bool {
    if let Some(r) = duration(s) {
        if blank(r) {
            return true;
        }
    }
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t') {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    inactive_then(&s[i..], &|t: &[u8]| {
        if blank(t) {
            return true;
        }
        if let Some(t2) = t.strip_prefix(b"--") {
            return inactive_then(t2, &|t3: &[u8]| matches!(duration(t3), Some(r) if blank(r)));
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_lines() {
        let s = Src::new("CLOCK: [2024-01-01 Mon 10:00]--[2024-01-01 Mon 11:00] =>  1:00\n");
        assert!(s.clock_line_full(0));
        let s = Src::new("CLOCK: [2024-01-01 Mon 10:00]\n");
        assert!(s.clock_line_full(0));
        let s = Src::new("CLOCK: [2024-01-01 Mon 10:00]--[2024-01-01 Mon 11:00]\n");
        assert!(s.clock_line_full(0));
        let s = Src::new("CLOCK: => 1:00");
        assert!(s.clock_line_full(0));
        let s = Src::new("CLOCK: foo");
        assert!(!s.clock_line_full(0));
    }

    #[test]
    fn lines() {
        let s = Src::new("a\nbb\n");
        assert_eq!(s.next_line(0), 2);
        assert_eq!(s.next_line(2), 5);
        assert_eq!(s.next_line(5), 5);
        assert_eq!(s.trim_end(0, 5), 5);
        let s = Src::new("a\n\n\n");
        assert_eq!(s.trim_end(0, 4), 2);
        assert_eq!(s.indent_col(0), 0);
        let s = Src::new("\t x");
        assert_eq!(s.indent_col(0), 9);
        let s = Src::new(" \tx");
        assert_eq!(s.indent_col(0), 9);
    }
}
