//! Lexical closing-delimiter positions, shared by all element searches.
//!
//! Index closers even inside raw blocks: the caller's range determines which
//! ones are visible. Org uses the first matching closer, not balanced pairs.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use super::text::{is_space, Src};

type Names<'a, T> = HashMap<(usize, Cow<'a, str>), Vec<T>>;

#[derive(Default)]
pub(super) struct DelimiterIndex<'a> {
    blocks: Names<'a, usize>,
    drawers: Vec<usize>,
    dynamic: Vec<usize>,
    dynamic_colon: Vec<usize>,
    latex: Names<'a, Range<usize>>,
}

/// Match `eq_ci`, including its equal-byte-length requirement for Unicode.
fn key(name: &str) -> (usize, Cow<'_, str>) {
    let folded = if name
        .bytes()
        .any(|b| !b.is_ascii() || b.is_ascii_uppercase())
    {
        Cow::Owned(name.to_lowercase())
    } else {
        Cow::Borrowed(name)
    };
    (name.len(), folded)
}

impl<'a> DelimiterIndex<'a> {
    pub(super) fn new(src: Src<'a>) -> Self {
        let mut index = Self::default();
        let mut pos = 0;
        while pos < src.len() {
            let end = src.eol(pos);
            let start = src.skip_blank_to(pos, end);
            let trimmed = src.skip_blank_back(end, start);
            let line = src.s(start..trimmed);
            if line.eq_ignore_ascii_case(":END:") {
                index.drawers.push(pos);
            } else if src.starts_ci(start, "#+END_") {
                let name = &line[6..];
                if !name.is_empty() && !name.chars().any(is_space) {
                    index.blocks.entry(key(name)).or_default().push(pos);
                }
            } else if line.eq_ignore_ascii_case("#+END:") {
                index.dynamic.push(pos);
                index.dynamic_colon.push(pos);
            } else if line.eq_ignore_ascii_case("#+END") {
                index.dynamic.push(pos);
            }

            // A LaTeX closer may be anywhere on a line, but only blanks can
            // follow it. At most one such match exists on each line.
            if line.ends_with('}') {
                if let Some(slash) = line.rfind('\\') {
                    let at = start + slash;
                    if src.starts_ci(at, "\\end{") {
                        let name = src.s(at + 5..trimmed - 1);
                        if !name.is_empty()
                            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'*')
                        {
                            index.latex.entry(key(name)).or_default().push(at..end);
                        }
                    }
                }
            }
            pos = end + 1;
        }
        index
    }

    pub(super) fn block(&self, from: usize, limit: usize, name: &str) -> Option<usize> {
        next(self.blocks.get(&key(name))?, from, limit)
    }

    pub(super) fn drawer(&self, from: usize, limit: usize) -> Option<usize> {
        next(&self.drawers, from, limit)
    }

    pub(super) fn dynamic(&self, from: usize, limit: usize, colon_only: bool) -> Option<usize> {
        next(
            if colon_only {
                &self.dynamic_colon
            } else {
                &self.dynamic
            },
            from,
            limit,
        )
    }

    pub(super) fn latex(&self, from: usize, limit: usize, name: &str) -> Option<usize> {
        let positions = self.latex.get(&key(name))?;
        let at = positions.partition_point(|r| r.start < from);
        // Unlike block searches, LaTeX requires the entire match (including
        // trailing blanks) to fit within the container.
        positions.get(at).filter(|r| r.end <= limit).map(|r| r.end)
    }
}

/// Line searches constrain the start of the closer, not the end of its line.
fn next(positions: &[usize], from: usize, limit: usize) -> Option<usize> {
    let at = positions.partition_point(|&p| p < from);
    positions.get(at).copied().filter(|&p| p < limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_searches_match_the_original_scans() {
        let text = "text\n #+END_x \t\n#+end_X\n#+END_x extra\n:END:\n :end:\t\n:END:\r\n#+END:\n#+END\n#+END: extra\n\\end{Eq}  \nprefix \\END{EQ}\t\n\\end{eq} extra\n#+END_Ä\n#+END_İ\n#+END_i\u{307}\n#+END_foo\u{c}\n\\end{eq}";
        let src = Src::new(text);
        let index = DelimiterIndex::new(src);
        let boundaries: Vec<_> = (0..=text.len())
            .filter(|&p| text.is_char_boundary(p))
            .collect();
        for &from in &boundaries {
            for &limit in boundaries.iter().filter(|&&p| p >= from) {
                for name in ["x", "X", "absent", "ä", "Ä", "İ", "i\u{307}"] {
                    assert_eq!(
                        index.block(from, limit, name),
                        src.find_block_end(from, limit, name),
                        "{from}..{limit}: {name}"
                    );
                }
                assert_eq!(
                    index.drawer(from, limit),
                    src.find_line(from, limit, |q| src.end_drawer_line(q))
                );
                for colon_only in [false, true] {
                    let expected = src.find_line(from, limit, |q| {
                        let i = src.skip_blank(q);
                        if !src.starts_ci(i, "#+END") {
                            return false;
                        }
                        let mut j = i + 5;
                        if src.at(j) == b':' {
                            j += 1;
                        } else if colon_only {
                            return false;
                        }
                        src.rest_blank(j)
                    });
                    assert_eq!(index.dynamic(from, limit, colon_only), expected);
                }
                assert_eq!(
                    index.latex(from, limit, "eq"),
                    src.find_latex_end(from, limit, "eq"),
                    "{from}..{limit}"
                );
            }
        }
    }
}
