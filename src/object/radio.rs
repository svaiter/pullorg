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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RadioTargets<'a> {
    names: Vec<CowStr<'a>>,
    edges: HashMap<(usize, Letter), usize>,
    /// Candidate indices at the end of each literal prefix.
    candidates: Vec<Vec<usize>>,
    suffixes: Vec<Vec<Option<char>>>,
    /// ASCII characters that can start a target, for a quick scan.
    ascii_starts: [bool; 128],
}

impl Default for RadioTargets<'_> {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            edges: HashMap::new(),
            candidates: Vec::new(),
            suffixes: Vec::new(),
            ascii_starts: [false; 128],
        }
    }
}

impl<'a> RadioTargets<'a> {
    /// Build the matcher. `names` gives the priority between targets of the
    /// same length (`org-update-radio-target-regexp` lists the most recent
    /// first); empty names and repeats are dropped, and longer targets are
    /// tried first.
    pub(crate) fn new(mut names: Vec<CowStr<'a>>) -> Self {
        let mut seen = std::collections::HashSet::new();
        names.retain(|name| !name.is_empty() && seen.insert(name.clone()));
        if names.is_empty() {
            return Self::default();
        }
        names.sort_by_cached_key(|t| std::cmp::Reverse(t.chars().count()));
        let mut index = Self {
            names,
            candidates: vec![Vec::new()],
            ..Self::default()
        };
        for (id, name) in index.names.iter().enumerate() {
            if let Some(first) = name.chars().next() {
                if first == ' ' {
                    // A space matches any whitespace.
                    for c in 0..128u8 {
                        index.ascii_starts[c as usize] |= is_ws(c as char);
                    }
                }
                for c in std::iter::once(first).chain(first.to_lowercase()) {
                    if c.is_ascii() {
                        index.ascii_starts[c.to_ascii_lowercase() as usize] = true;
                        index.ascii_starts[c.to_ascii_uppercase() as usize] = true;
                    }
                }
            }
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

    /// Whether some target's text occurs anywhere in `text`, ignoring the
    /// boundaries a radio link needs. A container where this is false parses
    /// the same with or without these targets.
    pub(crate) fn occurs_in(&self, text: &str) -> bool {
        if self.names.is_empty() {
            return false;
        }
        text.char_indices().any(|(start, c)| {
            // A non-ASCII character can fold onto an ASCII one (the Kelvin
            // sign), so only ASCII characters are filtered.
            if c.is_ascii() && !self.ascii_starts[c as usize] {
                return false;
            }
            let (mut node, mut pos) = (0, start);
            loop {
                if self.candidates[node]
                    .iter()
                    .any(|&id| suffix_end(text, pos, &self.suffixes[id]).is_some())
                {
                    return true;
                }
                let Some(c) = text[pos..].chars().next() else {
                    return false;
                };
                let Some(&next) = self.edges.get(&(node, Letter(c))) else {
                    return false;
                };
                node = next;
                pos += c.len_utf8();
            }
        })
    }
}

/// End of `suffix` matched at `pos`: literal characters compare
/// case-insensitively, `None` matches a run of whitespace.
fn suffix_end(text: &str, mut pos: usize, suffix: &[Option<char>]) -> Option<usize> {
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
    Some(pos)
}

/// [`suffix_end`] followed by the right boundary of a radio link.
fn match_suffix(text: &str, pos: usize, suffix: &[Option<char>]) -> Option<(usize, usize)> {
    let pos = suffix_end(text, pos, suffix)?;
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

    /// The matcher this index replaced: every target in priority order,
    /// character by character.
    fn linear_match(names: &[CowStr<'_>], text: &str) -> Option<(usize, usize, usize)> {
        names.iter().enumerate().find_map(|(id, name)| {
            let mut pieces = Vec::new();
            for c in name.chars() {
                if c != ' ' {
                    pieces.push(Some(c));
                } else if pieces.last() != Some(&None) {
                    pieces.push(None);
                }
            }
            match_suffix(text, 0, &pieces).map(|(end, after)| (id, end, after))
        })
    }

    #[test]
    fn index_matches_linear_matcher() {
        const CHARS: &[char] = &[
            'a', 'A', 'b', 'k', 'K', '\u{212a}', 'i', 'İ', 'é', 'É', ' ', ' ', '\t', '\n',
            '\u{a0}', '/', '.', '日',
        ];
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        for _ in 0..5000 {
            let mut word = |max: usize| -> String {
                let len = 1 + next(max);
                (0..len).map(|_| CHARS[next(CHARS.len())]).collect()
            };
            let count = 1 + word(5).chars().count();
            let names: Vec<CowStr<'_>> = (0..count).map(|_| word(4).into()).collect();
            let text = word(8);
            let matcher = RadioTargets::new(names);
            for (start, _) in text.char_indices() {
                let rest = &text[start..];
                assert_eq!(
                    matcher.match_at(rest),
                    linear_match(matcher.names(), rest),
                    "{:?} in {rest:?}",
                    matcher.names()
                );
            }
            let any = text
                .char_indices()
                .any(|(start, _)| linear_match(matcher.names(), &text[start..]).is_some());
            assert!(
                !any || matcher.occurs_in(&text),
                "{:?} in {text:?}",
                matcher.names()
            );
        }
    }

    #[test]
    fn new_drops_empty_and_repeated_names_and_sorts_longest_first() {
        let matcher = RadioTargets::new(
            ["a", "", "abc", "ab", "a", "xyz"]
                .into_iter()
                .map(CowStr::from)
                .collect(),
        );
        assert_eq!(matcher.names(), ["abc", "xyz", "ab", "a"]);
    }
}
