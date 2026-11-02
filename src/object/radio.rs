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
    pub(crate) fn new(names: Vec<CowStr<'a>>) -> Self {
        let mut index = Self::default();
        index.replace_names(names);
        index
    }

    /// Reuse the existing trie when the refreshed names have the same priority.
    pub(crate) fn replace_names(&mut self, mut names: Vec<CowStr<'a>>) {
        let mut seen = std::collections::HashSet::new();
        names.retain(|name| !name.is_empty() && seen.insert(name.clone()));
        names.sort_by_cached_key(|t| std::cmp::Reverse(t.chars().count()));
        if names == self.names {
            return;
        }
        *self = Self {
            names,
            ..Self::default()
        };
        if self.names.is_empty() {
            return;
        }
        self.candidates.push(Vec::new());
        for (id, name) in self.names.iter().enumerate() {
            if let Some(first) = name.chars().next() {
                if first == ' ' {
                    // A space matches any whitespace.
                    for c in 0..128u8 {
                        self.ascii_starts[c as usize] |= is_ws(c as char);
                    }
                }
                for c in std::iter::once(first).chain(first.to_lowercase()) {
                    if c.is_ascii() {
                        self.ascii_starts[c.to_ascii_lowercase() as usize] = true;
                        self.ascii_starts[c.to_ascii_uppercase() as usize] = true;
                    }
                }
            }
            let end = name.find(' ').unwrap_or(name.len());
            let mut node = 0;
            for c in name[..end].chars() {
                node = *self.edges.entry((node, Letter(c))).or_insert_with(|| {
                    self.candidates.push(Vec::new());
                    self.candidates.len() - 1
                });
            }
            self.candidates[node].push(id);
            let mut suffix = Vec::new();
            for c in name[end..].chars() {
                if c != ' ' {
                    suffix.push(Some(c));
                } else if suffix.last() != Some(&None) {
                    suffix.push(None);
                }
            }
            self.suffixes.push(suffix);
        }
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

    /// A conservative prefilter: a target's text matching somewhere (link
    /// boundaries aside) or an exhausted budget means the object parser must
    /// decide. The budget bounds the characters read across all overlapping
    /// starts, trie walks and suffix checks linearly in the segment length.
    pub(crate) fn may_occur_in(&self, text: &str) -> bool {
        if self.names.is_empty() {
            return false;
        }
        // Linear in the segment, with room for a few full checks in short
        // segments.
        let mut budget = text.len() + 64;
        for (start, c) in text.char_indices() {
            // Non-ASCII characters may fold onto an ASCII start (e.g. Kelvin sign).
            if c.is_ascii() && !self.ascii_starts[c as usize] {
                continue;
            }
            let (mut node, mut pos) = (0, start);
            loop {
                for &id in &self.candidates[node] {
                    match suffix_within(text, pos, &self.suffixes[id], &mut budget) {
                        Some(false) => {}
                        // A match, or no budget left to rule one out.
                        Some(true) | None => return true,
                    }
                }
                let Some(c) = text[pos..].chars().next() else {
                    break;
                };
                if budget == 0 {
                    return true;
                }
                budget -= 1;
                let Some(&next) = self.edges.get(&(node, Letter(c))) else {
                    break;
                };
                node = next;
                pos += c.len_utf8();
            }
        }
        false
    }
}

/// Whether `suffix` matches at `pos`, like [`suffix_end`], spending one unit
/// of `budget` per character read; `None` when the budget runs out first.
fn suffix_within(
    text: &str,
    mut pos: usize,
    suffix: &[Option<char>],
    budget: &mut usize,
) -> Option<bool> {
    let mut next = |pos: usize| -> Option<Option<char>> {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        Some(text[pos..].chars().next())
    };
    for piece in suffix {
        let Some(c) = next(pos)? else {
            return Some(false);
        };
        match piece {
            Some(pc) if eq_fold(*pc, c) => pos += c.len_utf8(),
            None if is_ws(c) => {
                pos += c.len_utf8();
                while let Some(c) = text[pos..].chars().next().filter(|&c| is_ws(c)) {
                    next(pos)?;
                    pos += c.len_utf8();
                }
            }
            _ => return Some(false),
        }
    }
    Some(true)
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
                !any || matcher.may_occur_in(&text),
                "{:?} in {text:?}",
                matcher.names()
            );
        }
    }

    #[test]
    fn near_match_falls_back_to_the_parser() {
        let prefix = "a".repeat(16_384);
        let matcher = RadioTargets::new(vec![format!("{prefix}b").into()]);
        assert!(matcher.may_occur_in(&prefix));
        assert!(!matcher.may_occur_in("ordinary words"));
        // The rest of a multi-word target is checked, not only its first word.
        let matcher = RadioTargets::new(vec!["a b".into()]);
        assert!(!matcher.may_occur_in("a c, a, b a"));
        assert!(matcher.may_occur_in("x A \t B y"));
    }

    #[test]
    fn unchanged_names_reuse_the_trie() {
        let mut matcher = RadioTargets::new(vec!["abc".into(), "k".into()]);
        let original = matcher.candidates.as_ptr();
        matcher.replace_names(vec!["k".into(), "abc".into(), "k".into(), "".into()]);
        assert_eq!(matcher.candidates.as_ptr(), original);
        assert_eq!(matcher.names(), ["abc", "k"]);
        matcher.replace_names(vec!["other".into()]);
        assert!(matcher.match_at("abc").is_none());
        assert!(matcher.match_at("other").is_some());
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
