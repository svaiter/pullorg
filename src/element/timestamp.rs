//! Timestamp parsing, following `org-element-timestamp-parser`.
//!
//! Used by the element parser for clock and planning lines.

use crate::event::{
    DateTime, Repeater, RepeaterKind, TimeUnit, Timestamp, TimestampKind, Warning, WarningKind,
};

/// `\d{4}-\d{2}-\d{2}` at `p`.
fn date_at(b: &[u8], p: usize) -> bool {
    p + 10 <= b.len()
        && b[p..p + 4].iter().all(u8::is_ascii_digit)
        && b[p + 4] == b'-'
        && b[p + 5..p + 7].iter().all(u8::is_ascii_digit)
        && b[p + 7] == b'-'
        && b[p + 8..p + 10].iter().all(u8::is_ascii_digit)
}

/// `[[<](\d{4}-\d{2}-\d{2}(?: .*?)?)[]>]` at `p`. Returns the end of the
/// match (after the closing bracket).
fn ts_both_at(b: &[u8], p: usize) -> Option<usize> {
    if p >= b.len() || !matches!(b[p], b'[' | b'<') || !date_at(b, p + 1) {
        return None;
    }
    let q = p + 11;
    match b.get(q) {
        Some(b']') | Some(b'>') => Some(q + 1),
        Some(b' ') => {
            let mut k = q + 1;
            while k < b.len() && b[k] != b'\n' {
                if matches!(b[k], b']' | b'>') {
                    return Some(k + 1);
                }
                k += 1;
            }
            None
        }
        _ => None,
    }
}

/// Diary timestamp `<%%(?:\([^>\n]+\))([^\n>]*)>` at `p`. Returns the range
/// of group 1 (the part after the sexp) and the match end.
fn diary_at(b: &[u8], p: usize) -> Option<(usize, usize, usize)> {
    if !b[p..].starts_with(b"<%%(") {
        return None;
    }
    let mut q = p + 4;
    while q < b.len() && b[q] != b'>' && b[q] != b'\n' {
        q += 1;
    }
    if q >= b.len() || b[q] != b'>' {
        return None;
    }
    // Greedy `[^>\n]+` then `)`: the last `)` before `q`, at index >= p+5.
    let close = (p + 5..q).rev().find(|&k| b[k] == b')')?;
    Some((close + 1, q, q + 1))
}

/// Match `[012]?[0-9]:[0-5][0-9]` at `i`, returning the hour digits range end
/// and the match end.
fn time_at(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let try_from = |h_end: usize| -> Option<(usize, usize)> {
        if h_end + 3 <= b.len()
            && b[h_end] == b':'
            && (b'0'..=b'5').contains(&b[h_end + 1])
            && b[h_end + 2].is_ascii_digit()
        {
            Some((h_end, h_end + 3))
        } else {
            None
        }
    };
    if i < b.len() && matches!(b[i], b'0'..=b'2') && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
        if let Some(r) = try_from(i + 2) {
            return Some(r);
        }
    }
    if i < b.len() && b[i].is_ascii_digit() {
        return try_from(i + 1);
    }
    None
}

fn num(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |acc, d| {
        acc.saturating_mul(10).saturating_add((d - b'0') as u32)
    })
}

/// Time range `[012]?[0-9]:[0-5][0-9]-([012]?[0-9]):([0-5][0-9])` in `s`
/// (leftmost match). Returns `(hour_end, minute_end)`.
fn time_range(s: &[u8]) -> Option<(u8, u8)> {
    for i in 0..s.len() {
        if let Some((_, e)) = time_at(s, i) {
            if s.get(e) == Some(&b'-') {
                if let Some((he, me)) = time_at(s, e + 1) {
                    let h = num(&s[e + 1..he]);
                    let m = num(&s[he + 1..me]);
                    return Some((h as u8, m as u8));
                }
            }
        }
    }
    None
}

/// `org-parse-time-string` with `nodefault`: leftmost
/// `(\d{4})-(\d{2})-(\d{2})( +[^]+0-9>\r\n -]+)?( +(\d{1,2}):(\d{2}))?`.
fn parse_time_string(s: &[u8]) -> Option<DateTime> {
    let p = (0..s.len()).find(|&p| date_at(s, p))?;
    let year = num(&s[p..p + 4]) as u16;
    let month = num(&s[p + 5..p + 7]) as u8;
    let day = num(&s[p + 8..p + 10]) as u8;
    let mut i = p + 10;
    // Optional day name.
    let spaces = |i: usize| {
        let mut j = i;
        while j < s.len() && s[j] == b' ' {
            j += 1;
        }
        j
    };
    let j = spaces(i);
    if j > i {
        let mut k = j;
        while k < s.len()
            && !matches!(
                s[k],
                b']' | b'+' | b'0'..=b'9' | b'>' | b'\r' | b'\n' | b' ' | b'-'
            )
        {
            k += 1;
        }
        if k > j {
            i = k;
        }
    }
    let mut hour = None;
    let mut minute = None;
    let j = spaces(i);
    if j > i {
        let d = {
            let mut d = j;
            while d < s.len() && s[d].is_ascii_digit() && d - j < 2 {
                d += 1;
            }
            d
        };
        // \d{1,2} greedy, then ':' \d{2}; backtrack to one digit if needed.
        for h_end in (j + 1..=d).rev() {
            if h_end + 3 <= s.len()
                && s[h_end] == b':'
                && s[h_end + 1].is_ascii_digit()
                && s[h_end + 2].is_ascii_digit()
            {
                hour = Some(num(&s[j..h_end]) as u8);
                minute = Some(num(&s[h_end + 1..h_end + 3]) as u8);
                break;
            }
        }
    }
    Some(DateTime {
        year,
        month,
        day,
        hour,
        minute,
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

/// Leftmost `(\+|\+\+|\.\+)(\d+)([hdwmy])` in `raw`.
fn repeater(raw: &[u8]) -> Option<Repeater> {
    for i in 0..raw.len() {
        let cands: [(&[u8], RepeaterKind); 3] = [
            (b"+", RepeaterKind::Cumulate),
            (b"++", RepeaterKind::CatchUp),
            (b".+", RepeaterKind::Restart),
        ];
        for (pre, kind) in cands {
            if raw[i..].starts_with(pre) {
                let d0 = i + pre.len();
                let mut d = d0;
                while d < raw.len() && raw[d].is_ascii_digit() {
                    d += 1;
                }
                if d > d0 {
                    if let Some(u) = raw.get(d).and_then(|&c| unit(c)) {
                        return Some(Repeater {
                            kind,
                            value: num(&raw[d0..d]),
                            unit: u,
                        });
                    }
                }
            }
        }
    }
    None
}

/// Leftmost `(-)?-(\d+)([hdwmy])` in `raw`.
fn warning(raw: &[u8]) -> Option<Warning> {
    let digits_unit = |d0: usize| -> Option<(u32, TimeUnit)> {
        let mut d = d0;
        while d < raw.len() && raw[d].is_ascii_digit() {
            d += 1;
        }
        if d == d0 {
            return None;
        }
        let u = raw.get(d).and_then(|&c| unit(c))?;
        Some((num(&raw[d0..d]), u))
    };
    for i in 0..raw.len() {
        if raw[i] != b'-' {
            continue;
        }
        if raw.get(i + 1) == Some(&b'-') {
            if let Some((v, u)) = digits_unit(i + 2) {
                return Some(Warning {
                    kind: WarningKind::First,
                    value: v,
                    unit: u,
                });
            }
        }
        if let Some((v, u)) = digits_unit(i + 1) {
            return Some(Warning {
                kind: WarningKind::All,
                value: v,
                unit: u,
            });
        }
    }
    None
}

/// Parse a timestamp starting at byte `p` of `text`. Returns the timestamp
/// and the end of its raw value.
pub(crate) fn parse_timestamp(text: &str, p: usize) -> Option<(Timestamp<'_>, usize)> {
    let b = text.as_bytes();
    if p >= b.len() {
        return None;
    }
    let diary = diary_at(b, p);
    if diary.is_none() && ts_both_at(b, p).is_none() {
        return None;
    }
    let active = b[p] == b'<';
    // Raw value: `([<[](%%)?.*?)[]>](?:--(TS))?`.
    let mut g1s = p + 1;
    let diaryp = b[p + 1..].starts_with(b"%%");
    if diaryp {
        g1s += 2;
    }
    let mut close = g1s;
    while close < b.len() && !matches!(b[close], b']' | b'>' | b'\n') {
        close += 1;
    }
    if close >= b.len() || b[close] == b'\n' {
        return None;
    }
    let g1_end = close;
    let mut raw_end = close + 1;
    let mut date_end = None;
    if b[raw_end..].starts_with(b"--") {
        if let Some(e) = ts_both_at(b, raw_end + 2) {
            date_end = Some(raw_end + 2..e);
            raw_end = e;
        }
    }
    let raw = &text[p..raw_end];
    let date_start: &[u8] = match diary {
        Some((s, e, _)) if diaryp => &b[s..e],
        _ => &b[p..g1_end],
    };
    let trange = time_range(date_start);
    let kind = if diaryp {
        TimestampKind::Diary
    } else if active && (date_end.is_some() || trange.is_some()) {
        TimestampKind::ActiveRange
    } else if active {
        TimestampKind::Active
    } else if date_end.is_some() || trange.is_some() {
        TimestampKind::InactiveRange
    } else {
        TimestampKind::Inactive
    };
    let (start, end, rep, warn) = if diaryp {
        (None, None, None, None)
    } else {
        let start = parse_time_string(date_start);
        let end = if let Some(r) = &date_end {
            parse_time_string(&b[r.clone()])
        } else if let (Some((h, m)), Some(s)) = (trange, start) {
            Some(DateTime {
                hour: Some(h),
                minute: Some(m),
                ..s
            })
        } else {
            None
        };
        (
            start,
            end,
            repeater(raw.as_bytes()),
            warning(raw.as_bytes()),
        )
    };
    Some((
        Timestamp {
            kind,
            raw: raw.into(),
            start,
            end,
            repeater: rep,
            warning: warn,
        },
        raw_end,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple() {
        let (ts, e) = parse_timestamp("<2024-01-02 Tue 10:30 +1w -2d>", 0).unwrap();
        assert_eq!(e, 30);
        assert_eq!(ts.kind, TimestampKind::Active);
        let s = ts.start.unwrap();
        assert_eq!(
            (s.year, s.month, s.day, s.hour, s.minute),
            (2024, 1, 2, Some(10), Some(30))
        );
        assert_eq!(ts.end, None);
        assert_eq!(ts.repeater.unwrap().kind, RepeaterKind::Cumulate);
        assert_eq!(ts.warning.unwrap().value, 2);
    }

    #[test]
    fn ranges() {
        let (ts, _) = parse_timestamp("[2024-01-02 Tue 10:00-11:30]", 0).unwrap();
        assert_eq!(ts.kind, TimestampKind::InactiveRange);
        assert_eq!(ts.end.unwrap().hour, Some(11));
        assert_eq!(ts.end.unwrap().minute, Some(30));
        let (ts, e) = parse_timestamp("[2024-01-02 Tue]--[2024-01-03 Wed] x", 0).unwrap();
        assert_eq!(e, 34);
        assert_eq!(ts.kind, TimestampKind::InactiveRange);
        assert_eq!(ts.end.unwrap().day, 3);
        let (ts, _) = parse_timestamp("<%%(diary-float t 4 2)>", 0).unwrap();
        assert_eq!(ts.kind, TimestampKind::Diary);
        assert!(parse_timestamp("<2024-1-02>", 0).is_none());
        let (ts, _) = parse_timestamp("<2024-01-02 Tue .+1d>", 0).unwrap();
        assert_eq!(ts.repeater.unwrap().kind, RepeaterKind::Restart);
        let (ts, _) = parse_timestamp("<2024-01-02 Tue ++1d --3w>", 0).unwrap();
        assert_eq!(ts.repeater.unwrap().kind, RepeaterKind::CatchUp);
        assert_eq!(ts.warning.unwrap().kind, WarningKind::First);
    }

    /// Expected values printed by `org-element-timestamp-parser` (Org 9.8).
    #[test]
    fn matches_emacs_samples() {
        let cases = [
            ("<2024-01-02 Tue>", "active s=2024-1-2 nil:nil e=2024-1-2 nil:nil rep=nil/nil/nil warn=nil/nil/nil"),
            ("[2024-01-02 Tue 9:05]", "inactive s=2024-1-2 9:5 e=2024-1-2 9:5 rep=nil/nil/nil warn=nil/nil/nil"),
            ("<2024-01-02 Tue 10:00-11:30 +1w>", "active-range s=2024-1-2 10:0 e=2024-1-2 11:30 rep=cumulate/1/week warn=nil/nil/nil"),
            ("<2024-01-02 Tue 10:00>--<2024-01-05 Fri 12:00>", "active-range s=2024-1-2 10:0 e=2024-1-5 12:0 rep=nil/nil/nil warn=nil/nil/nil"),
            ("[2024-01-02]--[2024-01-03]", "inactive-range s=2024-1-2 nil:nil e=2024-1-3 nil:nil rep=nil/nil/nil warn=nil/nil/nil"),
            ("<2024-01-02 Tue .+2d/3d -1w>", "active s=2024-1-2 nil:nil e=2024-1-2 nil:nil rep=restart/2/day warn=all/1/week"),
            ("<2024-01-02 Tue ++1m --2d>", "active s=2024-1-2 nil:nil e=2024-1-2 nil:nil rep=catch-up/1/month warn=first/2/day"),
            ("<%%(diary-float t 4 2)>", "diary s=nil-nil-nil nil:nil e=nil-nil-nil nil:nil rep=nil/nil/nil warn=nil/nil/nil"),
            ("<2024-01-02 10:30>", "active s=2024-1-2 10:30 e=2024-1-2 10:30 rep=nil/nil/nil warn=nil/nil/nil"),
            ("[2024-01-02 Tue>", "inactive s=2024-1-2 nil:nil e=2024-1-2 nil:nil rep=nil/nil/nil warn=nil/nil/nil"),
            ("<2024-01-02 Tue 100:00>", "active s=2024-1-2 nil:nil e=2024-1-2 nil:nil rep=nil/nil/nil warn=nil/nil/nil"),
            ("<2024-01-02 Tue 7:00-8:00>", "active-range s=2024-1-2 7:0 e=2024-1-2 8:0 rep=nil/nil/nil warn=nil/nil/nil"),
            ("<2024-01-02 Mon 12:00 -3d>", "active s=2024-1-2 12:0 e=2024-1-2 12:0 rep=nil/nil/nil warn=all/3/day"),
        ];
        let o = |v: Option<String>| v.unwrap_or_else(|| "nil".into());
        let dt = |d: Option<DateTime>| match d {
            None => "nil-nil-nil nil:nil".to_string(),
            Some(d) => format!(
                "{}-{}-{} {}:{}",
                d.year,
                d.month,
                d.day,
                o(d.hour.map(|h| h.to_string())),
                o(d.minute.map(|m| m.to_string()))
            ),
        };
        let unit = |u: TimeUnit| match u {
            TimeUnit::Hour => "hour",
            TimeUnit::Day => "day",
            TimeUnit::Week => "week",
            TimeUnit::Month => "month",
            TimeUnit::Year => "year",
        };
        for (input, expected) in cases {
            let (ts, end) = parse_timestamp(input, 0).unwrap();
            assert_eq!(end, input.len());
            assert_eq!(ts.raw, input);
            let kind = match ts.kind {
                TimestampKind::Active => "active",
                TimestampKind::Inactive => "inactive",
                TimestampKind::ActiveRange => "active-range",
                TimestampKind::InactiveRange => "inactive-range",
                TimestampKind::Diary => "diary",
            };
            let rep = match ts.repeater {
                None => "nil/nil/nil".to_string(),
                Some(r) => format!(
                    "{}/{}/{}",
                    match r.kind {
                        RepeaterKind::Cumulate => "cumulate",
                        RepeaterKind::CatchUp => "catch-up",
                        RepeaterKind::Restart => "restart",
                    },
                    r.value,
                    unit(r.unit)
                ),
            };
            let warn = match ts.warning {
                None => "nil/nil/nil".to_string(),
                Some(w) => format!(
                    "{}/{}/{}",
                    match w.kind {
                        WarningKind::All => "all",
                        WarningKind::First => "first",
                    },
                    w.value,
                    unit(w.unit)
                ),
            };
            // Emacs reports the start as the end of non-range timestamps.
            let got = format!(
                "{} s={} e={} rep={} warn={}",
                kind,
                dt(ts.start),
                dt(ts.end.or(ts.start)),
                rep,
                warn
            );
            assert_eq!(got, expected, "{}", input);
        }
    }
}
