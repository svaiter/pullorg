//! Timestamps (`org-element-timestamp-parser`).

use super::{Kind, Obj, Scanner};
use crate::event::{
    DateTime, Event, Repeater, RepeaterKind, TimeUnit, Timestamp, TimestampKind, Warning,
    WarningKind,
};

fn digits(b: &[u8], i: usize, n: usize) -> bool {
    b.len() >= i + n && b[i..i + n].iter().all(u8::is_ascii_digit)
}

fn number(b: &[u8]) -> u32 {
    b.iter()
        .try_fold(0u32, |acc, d| {
            acc.checked_mul(10)?.checked_add(u32::from(d - b'0'))
        })
        .unwrap_or(u32::MAX)
}

/// `org-ts-regexp-both` at the start of `b`: `[[<]YYYY-MM-DD\( .*?\)?[]>]`.
/// Returns the length of the match.
pub(super) fn ts_both(b: &[u8]) -> Option<usize> {
    if !matches!(b.first(), Some(b'<' | b'[')) || !digits(b, 1, 4) || b.get(5) != Some(&b'-') {
        return None;
    }
    if !digits(b, 6, 2) || b.get(8) != Some(&b'-') || !digits(b, 9, 2) {
        return None;
    }
    match b.get(11) {
        Some(b']' | b'>') => Some(12),
        Some(b' ') => b[12..]
            .iter()
            .take_while(|&&c| c != b'\n')
            .position(|&c| c == b']' || c == b'>')
            .map(|i| 12 + i + 1),
        _ => None,
    }
}

/// `<%%(SEXP)REST>`: returns `(rest_start, length)`.
fn diary(b: &[u8]) -> Option<(usize, usize)> {
    if !b.starts_with(b"<%%(") {
        return None;
    }
    // `[^>\n]+` greedy, then backtrack to the last `)`.
    let stop = b[4..]
        .iter()
        .position(|&c| c == b'>' || c == b'\n')
        .map_or(b.len(), |i| 4 + i);
    let close = b
        .get(5..stop)?
        .iter()
        .rposition(|&c| c == b')')
        .map(|i| 5 + i)?;
    if b.get(stop) != Some(&b'>') {
        return None;
    }
    Some((close + 1, stop + 1))
}

/// `org-ts-regexp0` searched in `s`: date, optional day name, optional time.
fn parse_date(s: &[u8]) -> Option<DateTime> {
    (0..s.len()).find_map(|i| {
        let b = &s[i..];
        if !(digits(b, 0, 4)
            && b.get(4) == Some(&b'-')
            && digits(b, 5, 2)
            && b.get(7) == Some(&b'-')
            && digits(b, 8, 2))
        {
            return None;
        }
        let mut j = 10;
        // Day name: ` +[^]+0-9>\r\n -]+`.
        let spaces = b[j..].iter().take_while(|&&c| c == b' ').count();
        if spaces > 0 {
            let name = b[j + spaces..]
                .iter()
                .take_while(|&&c| {
                    !matches!(
                        c,
                        b']' | b'+' | b'0'..=b'9' | b'>' | b'\r' | b'\n' | b' ' | b'-'
                    )
                })
                .count();
            if name > 0 {
                j += spaces + name;
            }
        }
        // Time: ` +\([0-9]\{1,2\}\):\([0-9]\{2\}\)`.
        let mut time = None;
        let spaces = b[j..].iter().take_while(|&&c| c == b' ').count();
        if spaces > 0 {
            let k = j + spaces;
            let hl = b[k..]
                .iter()
                .take(2)
                .take_while(|c| c.is_ascii_digit())
                .count();
            if hl > 0 && b.get(k + hl) == Some(&b':') && digits(b, k + hl + 1, 2) {
                time = Some((
                    number(&b[k..k + hl]) as u8,
                    number(&b[k + hl + 1..k + hl + 3]) as u8,
                ));
            }
        }
        Some(DateTime {
            year: number(&b[0..4]) as u16,
            month: number(&b[5..7]) as u8,
            day: number(&b[8..10]) as u8,
            hour: time.map(|t| t.0),
            minute: time.map(|t| t.1),
        })
    })
}

/// Match `[012]?[0-9]:[0-5][0-9]` at `i`; returns `(hour, minute, end)`.
fn hhmm(s: &[u8], i: usize) -> Vec<(u8, u8, usize)> {
    let mut res = Vec::new();
    // `[012]?` is greedy: first try with it.
    for skip in [1usize, 0] {
        if skip == 1 && !matches!(s.get(i), Some(b'0'..=b'2')) {
            continue;
        }
        let h = i + skip;
        if s.get(h).is_some_and(u8::is_ascii_digit)
            && s.get(h + 1) == Some(&b':')
            && matches!(s.get(h + 2), Some(b'0'..=b'5'))
            && s.get(h + 3).is_some_and(u8::is_ascii_digit)
        {
            res.push((
                number(&s[i..h + 1]) as u8,
                number(&s[h + 2..h + 4]) as u8,
                h + 4,
            ));
        }
    }
    res
}

/// Search `[012]?[0-9]:[0-5][0-9]-\([012]?[0-9]\):\([0-5][0-9]\)`; returns the
/// end time.
fn time_range(s: &[u8]) -> Option<(u8, u8)> {
    (0..s.len()).find_map(|i| {
        hhmm(s, i).into_iter().find_map(|(_, _, e)| {
            if s.get(e) != Some(&b'-') {
                return None;
            }
            hhmm(s, e + 1).into_iter().next().map(|(h, m, _)| (h, m))
        })
    })
}

fn unit(c: u8) -> Option<TimeUnit> {
    Some(match c {
        b'h' => TimeUnit::Hour,
        b'd' => TimeUnit::Day,
        b'w' => TimeUnit::Week,
        b'm' => TimeUnit::Month,
        b'y' => TimeUnit::Year,
        _ => return None,
    })
}

/// `[0-9]+[hdwmy]` at `i`: `(value, unit, end)`.
fn value_unit(s: &[u8], i: usize) -> Option<(u32, TimeUnit, usize)> {
    let n = s[i.min(s.len())..]
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if n == 0 {
        return None;
    }
    let u = unit(*s.get(i + n)?)?;
    Some((number(&s[i..i + n]), u, i + n + 1))
}

fn repeater(s: &[u8]) -> Option<Repeater> {
    (0..s.len()).find_map(|i| {
        let candidates: [(&[u8], RepeaterKind); 3] = [
            (b"+", RepeaterKind::Cumulate),
            (b"++", RepeaterKind::CatchUp),
            (b".+", RepeaterKind::Restart),
        ];
        candidates.iter().find_map(|(mark, kind)| {
            if !s[i..].starts_with(mark) {
                return None;
            }
            let (value, unit, _) = value_unit(s, i + mark.len())?;
            Some(Repeater {
                kind: *kind,
                value,
                unit,
            })
        })
    })
}

fn warning(s: &[u8]) -> Option<Warning> {
    (0..s.len()).find_map(|i| {
        if s[i] != b'-' {
            return None;
        }
        if s.get(i + 1) == Some(&b'-') {
            if let Some((value, unit, _)) = value_unit(s, i + 2) {
                return Some(Warning {
                    kind: WarningKind::First,
                    value,
                    unit,
                });
            }
        }
        value_unit(s, i + 1).map(|(value, unit, _)| Warning {
            kind: WarningKind::All,
            value,
            unit,
        })
    })
}

impl<'p, 'a, 's> Scanner<'p, 'a, 's> {
    /// `org-element-timestamp-parser`.
    pub(super) fn timestamp(&self, p: usize) -> Option<Obj<'a>> {
        let b = self.rest(p).as_bytes();
        let diary_match = if ts_both(b).is_some() {
            None
        } else {
            Some(diary(b)?)
        };
        // Raw value: `[<[]\(%%\)?.*?[]>]\(?:--TS\)?`.
        let close = b[1..]
            .iter()
            .take_while(|&&c| c != b'\n')
            .position(|&c| c == b']' || c == b'>')?
            + 1;
        let is_diary = b[1..].starts_with(b"%%");
        let mut raw_end = close + 1;
        let mut date_end = None;
        if b[raw_end..].starts_with(b"--") {
            if let Some(n) = ts_both(&b[raw_end + 2..]) {
                date_end = Some(&b[raw_end + 2..raw_end + 2 + n]);
                raw_end += 2 + n;
            }
        }
        let raw = &b[..raw_end];
        let active = b[0] == b'<';
        let ts = if is_diary {
            // Diary timestamps have no date: their (optional) time is only
            // available through `raw`.
            let _ = diary_match;
            Timestamp {
                kind: TimestampKind::Diary,
                raw: self.borrow(p..p + raw_end),
                start: None,
                end: None,
                repeater: None,
                warning: None,
            }
        } else {
            let date_start = &b[..close];
            let start = parse_date(date_start)?;
            let tr = time_range(date_start);
            let end = match date_end {
                Some(de) => {
                    let e = parse_date(de)?;
                    Some(DateTime {
                        hour: e.hour.or(tr.map(|t| t.0)).or(start.hour),
                        minute: e.minute.or(tr.map(|t| t.1)).or(start.minute),
                        ..e
                    })
                }
                None => tr.map(|(h, m)| DateTime {
                    hour: Some(h),
                    minute: Some(m),
                    ..start
                }),
            };
            let kind = match (active, end.is_some()) {
                (true, true) => TimestampKind::ActiveRange,
                (true, false) => TimestampKind::Active,
                (false, true) => TimestampKind::InactiveRange,
                (false, false) => TimestampKind::Inactive,
            };
            Timestamp {
                kind,
                raw: self.borrow(p..p + raw_end),
                start: Some(start),
                end,
                repeater: repeater(raw),
                warning: warning(raw),
            }
        };
        Some(self.obj(p, p + raw_end, Kind::Leaf(Event::Timestamp(ts))))
    }
}
