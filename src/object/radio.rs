//! Document-wide radio link matcher. Index each literal prefix; after the
//! first space, Org permits one or more whitespace characters in its place.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use super::chars::{eq_fold, is_alnum, is_line_breakable, is_ws};
use crate::CowStr;

#[derive(Clone, Copy, Debug)]
struct Letter(char);

impl PartialEq for Letter {
    fn eq(&self, other: &Self) -> bool {
        eq_fold(self.0, other.0)
    }
}
impl Eq for Letter {}
impl Hash for Letter {
    fn hash<H: Hasher>(&self, h: &mut H) {
        for c in self.0.to_lowercase() {
            c.hash(h);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RadioTargets<'a> {
    names: Vec<CowStr<'a>>,
    edges: HashMap<(usize, Letter), usize>,
    /// Candidate indices at the end of each literal prefix.
    candidates: Vec<Vec<usize>>,
    suffixes: Vec<Vec<Option<char>>>,
}

impl<'a> RadioTargets<'a> {
    /// Names are in match priority order (longest first, latest on ties).
    pub(crate) fn new(names: Vec<CowStr<'a>>) -> Self {
        if names.is_empty() {
            return Self::default();
        }
        let mut index = Self {
            names,
            candidates: vec![Vec::new()],
            ..Self::default()
        };
        for (id, name) in index.names.iter().enumerate() {
            let end = name.find(' ').unwrap_or(name.len());
            let mut node = 0;
            for c in name[..end].chars() {
                node = *index.edges.entry((node, Letter(c))).or_insert_with(|| {
                    index.candidates.push(Vec::new());
                    index.candidates.len() - 1
                });
            }
            index.candidates[node].push(id);
            let mut suffix = Vec::new();
            for c in name[end..].chars() {
                if c != ' ' {
                    suffix.push(Some(c));
                } else if suffix.last() != Some(&None) {
                    suffix.push(None);
                }
            }
            index.suffixes.push(suffix);
        }
        index
    }

    pub(crate) fn names(&self) -> &[CowStr<'a>] {
        &self.names
    }

    /// Return the winning target and byte ends of its text and right boundary.
    pub(super) fn match_at(&self, text: &str) -> Option<(usize, usize, usize)> {
        if self.names.is_empty() {
            return None;
        }
        let mut best = None;
        let (mut node, mut pos) = (0, 0);
        loop {
            for &id in &self.candidates[node] {
                if best.is_some_and(|(rank, _, _)| rank <= id) {
                    break;
                }
                if let Some((end, after)) = match_suffix(text, pos, &self.suffixes[id]) {
                    best = Some((id, end, after));
                    break;
                }
            }
            let Some(c) = text[pos..].chars().next() else {
                break;
            };
            let Some(&next) = self.edges.get(&(node, Letter(c))) else {
                break;
            };
            node = next;
            pos += c.len_utf8();
        }
        best
    }
}

fn match_suffix(text: &str, mut pos: usize, suffix: &[Option<char>]) -> Option<(usize, usize)> {
    for piece in suffix {
        let mut chars = text[pos..].chars();
        let c = chars.next()?;
        match piece {
            Some(pc) if eq_fold(*pc, c) => pos += c.len_utf8(),
            None if is_ws(c) => {
                pos += c.len_utf8();
                pos += chars
                    .take_while(|&c| is_ws(c))
                    .map(char::len_utf8)
                    .sum::<usize>();
            }
            _ => return None,
        }
    }
    match text[pos..].chars().next() {
        None | Some('\n') => Some((pos, pos)),
        Some(c) if !is_alnum(c) || is_line_breakable(c) => Some((pos, pos + c.len_utf8())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_prefixes_keep_target_priority_and_org_boundaries() {
        let matcher = RadioTargets::new(
            ["a/b", "a\u{a0}b", "a b", "a", "İ", "K"]
                .into_iter()
                .map(CowStr::from)
                .collect(),
        );
        for (text, expected) in [
            ("a/b!", Some("a/b")),
            ("A/c", Some("a")),
            ("a\u{a0}B.", Some("a\u{a0}b")),
            ("A\u{200b} b!", Some("a b")),
            ("a\nb", Some("a b")),
            ("abc", None),
            ("İ!", Some("İ")),
            ("i\u{307}!", None),
            ("K.", Some("K")),
        ] {
            let actual = matcher
                .match_at(text)
                .map(|(i, _, _)| matcher.names()[i].as_ref());
            assert_eq!(actual, expected, "{text:?}");
        }
    }
}
