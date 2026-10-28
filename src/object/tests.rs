//! Tests for the object parser.
//!
//! Most expectations are written in a compact S-expression format produced
//! by [`render`], which mirrors a dump of the `org-element` tree Emacs
//! produces (0-based character positions, `end` excluding trailing blanks).
//! They were checked against Emacs 30 / Org 9.8.3.

use std::fmt::Write;
use std::ops::Range;

use super::{parse_objects, Restriction};
use crate::element::Settings;
use crate::event::*;
use crate::options::SubSuperscripts;
use crate::strings::CowStr;

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn opt(s: &Option<CowStr<'_>>) -> String {
    s.as_deref().map_or_else(|| "-".to_string(), quote)
}

/// Settings for a snippet: radio targets (`<<<...>>>`) and `#+LINK:` lines,
/// as the element parser would collect them.
fn snippet_settings(src: &str) -> Settings<'_> {
    let mut settings = Settings::default();
    for (i, _) in src.match_indices("<<<") {
        let after = &src[i + 3..];
        let n = after.find(['<', '>', '\n', '\r']).unwrap_or(after.len());
        let t = &after[..n];
        if !t.is_empty()
            && after[n..].starts_with(">>>")
            && !t.starts_with([' ', '\t'])
            && !t.ends_with([' ', '\t'])
        {
            settings.radio_targets.push(CowStr::Borrowed(t));
        }
    }
    for line in src.lines() {
        if let Some(l) = line.strip_prefix("#+LINK: ") {
            let mut it = l.splitn(2, ' ');
            let k = it.next().unwrap();
            let v = it.next().unwrap_or("").trim();
            settings
                .link_abbreviations
                .push((CowStr::Borrowed(k), CowStr::Borrowed(v)));
        }
    }
    settings
}

/// The paragraph range of a snippet: everything after the `#+LINK:` lines.
fn paragraph_range(src: &str) -> Range<usize> {
    let mut start = 0;
    while src[start..].starts_with("#+LINK: ") {
        start += src[start..].find('\n').map_or(src.len() - start, |i| i + 1);
    }
    start..src.len()
}

fn parse<'a>(
    src: &'a str,
    settings: &Settings<'a>,
    r: Restriction,
) -> Vec<(Event<'a>, Range<usize>)> {
    let mut out = Vec::new();
    parse_objects(src, paragraph_range(src), r, settings, &mut out);
    out
}

fn tag_name(tag: &Tag<'_>) -> String {
    match tag {
        Tag::Bold => "bold".into(),
        Tag::Italic => "italic".into(),
        Tag::Underline => "underline".into(),
        Tag::StrikeThrough => "strike-through".into(),
        Tag::RadioTarget => "radio-target".into(),
        Tag::Subscript { use_brackets } => {
            format!("subscript {}", if *use_brackets { "t" } else { "nil" })
        }
        Tag::Superscript { use_brackets } => {
            format!("superscript {}", if *use_brackets { "t" } else { "nil" })
        }
        Tag::FootnoteReference { label } => format!("footnote-reference {} inline", opt(label)),
        Tag::Citation {
            style,
            prefix,
            suffix,
        } => {
            format!("citation {} {} {}", opt(style), opt(prefix), opt(suffix))
        }
        Tag::Link(l) => format!(
            "link {} {} {} {} {}",
            l.link_type,
            quote(&l.path),
            match l.format {
                LinkFormat::Plain => "plain",
                LinkFormat::Angle => "angle",
                LinkFormat::Bracket => "bracket",
            },
            opt(&l.search_option),
            opt(&l.application)
        ),
        t => panic!("unexpected tag {t:?}"),
    }
}

fn leaf(ev: &Event<'_>) -> String {
    match ev {
        Event::Code(s) => format!("code {}", quote(s)),
        Event::Verbatim(s) => format!("verbatim {}", quote(s)),
        Event::LatexFragment(s) => format!("latex-fragment {}", quote(s)),
        Event::StatisticsCookie(s) => format!("statistics-cookie {}", quote(s)),
        Event::Target(s) => format!("target {}", quote(s)),
        Event::Entity(e) => format!(
            "entity {} {}",
            e.name,
            if e.use_brackets { "t" } else { "nil" }
        ),
        Event::ExportSnippet { backend, value } => {
            format!("export-snippet {} {}", backend, quote(value))
        }
        Event::Macro { key, args } => {
            let mut s = format!("macro {key}");
            for a in args {
                s.push(' ');
                s.push_str(&quote(a));
            }
            s
        }
        Event::InlineSrcBlock {
            language,
            parameters,
            value,
        } => format!(
            "inline-src-block {} {} {}",
            language,
            opt(parameters),
            quote(value)
        ),
        Event::InlineBabelCall {
            call,
            inside_header,
            arguments,
            end_header,
        } => format!(
            "inline-babel-call {} {} {} {}",
            call,
            opt(inside_header),
            opt(arguments),
            opt(end_header)
        ),
        Event::FootnoteReference(l) => format!("footnote-reference {} standard", quote(l)),
        Event::CitationReference {
            key,
            prefix,
            suffix,
        } => {
            format!("citation-reference {} {} {}", key, opt(prefix), opt(suffix))
        }
        Event::LineBreak => "line-break".into(),
        Event::Timestamp(t) => format!(
            "timestamp {} {}",
            match t.kind {
                TimestampKind::Active => "active",
                TimestampKind::Inactive => "inactive",
                TimestampKind::ActiveRange => "active-range",
                TimestampKind::InactiveRange => "inactive-range",
                TimestampKind::Diary => "diary",
            },
            quote(&t.raw)
        ),
        e => panic!("unexpected event {e:?}"),
    }
}

/// Render events as an S-expression list similar to the `org-element` tree.
/// Also checks that ranges are consistent and that the text round-trips.
fn render_events(src: &str, range: Range<usize>, events: &[(Event<'_>, Range<usize>)]) -> String {
    let cpos = |b: usize| src[..b].chars().count();
    let mut out = String::new();
    let mut text = String::new();
    let mut stack: Vec<Range<usize>> = Vec::new();
    // Round trip: positions covered by top-level events must be contiguous.
    let mut cursor = range.start;
    let flush = |out: &mut String, text: &mut String| {
        if !text.is_empty() {
            if !out.is_empty() && !out.ends_with('(') {
                out.push(' ');
            }
            out.push_str(&quote(text));
            text.clear();
        }
    };
    let sep = |out: &mut String| {
        if !out.is_empty() && !out.ends_with('(') {
            out.push(' ');
        }
    };
    for (ev, r) in events {
        assert!(
            r.start <= r.end && r.end <= src.len(),
            "bad range {r:?} for {ev:?}"
        );
        if let Some(parent) = stack.last() {
            assert!(
                parent.start <= r.start && r.end <= parent.end,
                "{ev:?} {r:?} outside {parent:?}"
            );
        }
        let top = stack.is_empty();
        match ev {
            Event::Text(t) => {
                assert_eq!(&src[r.clone()], &**t);
                assert!(!t.is_empty() && !t.contains('\n'));
                text.push_str(t);
            }
            Event::SoftBreak => {
                assert_eq!(&src[r.clone()], "\n");
                text.push('\n');
            }
            Event::Start(tag) => {
                flush(&mut out, &mut text);
                sep(&mut out);
                write!(
                    out,
                    "({} {} {}",
                    tag_name(tag).split(' ').next().unwrap(),
                    cpos(r.start),
                    cpos(r.end)
                )
                .unwrap();
                let name = tag_name(tag);
                if let Some((_, props)) = name.split_once(' ') {
                    write!(out, " {props}").unwrap();
                }
                stack.push(r.clone());
            }
            Event::End(_) => {
                flush(&mut out, &mut text);
                let s = stack.pop().expect("unbalanced End");
                assert_eq!(&s, r);
                out.push(')');
            }
            ev => {
                flush(&mut out, &mut text);
                sep(&mut out);
                let l = leaf(ev);
                let (name, props) = l.split_once(' ').unwrap_or((&l, ""));
                write!(out, "({} {} {}", name, cpos(r.start), cpos(r.end)).unwrap();
                if !props.is_empty() {
                    write!(out, " {props}").unwrap();
                }
                out.push(')');
            }
        }
        // Round trip: top-level events tile the range exactly.
        if top {
            if !matches!(ev, Event::End(_)) {
                assert_eq!(r.start, cursor, "gap or overlap before {ev:?} {r:?}");
                cursor = r.end;
            }
        } else if let Event::End(_) = ev {
            if stack.is_empty() {
                cursor = r.end;
            }
        }
    }
    flush(&mut out, &mut text);
    assert!(stack.is_empty(), "unclosed containers");
    assert_eq!(cursor, range.end, "text not covered at the end");
    out
}

fn render_with(src: &str, settings: &Settings<'_>, r: Restriction) -> String {
    let events = parse(src, settings, r);
    render_events(src, paragraph_range(src), &events)
}

fn render(src: &str) -> String {
    render_with(src, &snippet_settings(src), Restriction::Standard)
}

/// Like [`render`], but understands a few element prefixes: `* ` (headline
/// title), `| ` (one-row table: cells joined with ` | `) and `- tag :: `
/// (item tag).
fn render_element(src: &str) -> String {
    let settings = snippet_settings(src);
    if let Some(title) = src.strip_prefix("* ") {
        let start = src.len() - title.len();
        let end = start + title.trim_end().len();
        let mut out = Vec::new();
        parse_objects(
            src,
            start..end,
            Restriction::HeadlineTitle,
            &settings,
            &mut out,
        );
        return render_events(src, start..end, &out);
    }
    if src.starts_with("| ") {
        let mut cells = Vec::new();
        let mut pos = 1;
        while pos < src.len() {
            let len = src[pos..].find('|').unwrap_or(src.len() - pos);
            let cell = &src[pos..pos + len];
            let s = pos + (cell.len() - cell.trim_start_matches([' ', '\t']).len());
            let e = pos + cell.trim_end_matches([' ', '\t']).len();
            let mut out = Vec::new();
            parse_objects(
                src,
                s..e.max(s),
                Restriction::TableCell,
                &settings,
                &mut out,
            );
            cells.push(render_events(src, s..e.max(s), &out));
            pos += len + 1;
        }
        return cells.join(" | ");
    }
    if let Some(rest) = src.strip_prefix("- ") {
        if let Some(i) = rest.find(" :: ") {
            let mut out = Vec::new();
            parse_objects(src, 2..2 + i, Restriction::ItemTag, &settings, &mut out);
            return render_events(src, 2..2 + i, &out);
        }
    }
    render(src)
}

#[track_caller]
fn check_element(src: &str, expected: &str) {
    assert_eq!(render_element(src), expected, "input: {src:?}");
}

#[track_caller]
fn check(src: &str, expected: &str) {
    assert_eq!(render(src), expected, "input: {src:?}");
}

/// Dump the rendering of the snippets in `$ORACLE_INPUT` (separated by
/// `\n#####\n`) to `$ORACLE_OUTPUT`, to compare with the Emacs dump.
#[test]
#[ignore]
fn oracle_dump() {
    let input = std::fs::read_to_string(std::env::var("ORACLE_INPUT").unwrap()).unwrap();
    let mut out = String::new();
    for s in input.split("\n#####\n") {
        out.push_str(&render_element(s));
        out.push('\n');
    }
    std::fs::write(std::env::var("ORACLE_OUTPUT").unwrap(), out).unwrap();
}

/// Expectations generated from the Emacs parse tree (Org 9.8.3).
mod oracle {
    use super::{check, check_element};

    #[test]
    fn emphasis_basic() {
        check(
            r#"**a* x*y* (/i/) "=v=" a=b= _u_ +s+ ~c~d ~c~. *a *b* c*"#,
            r#"(bold 0 4 "*a") " x*y* (" (italic 11 14 "i") ") \"" (verbatim 17 20 "v") "\" a=b= " (underline 27 30 "u") " " (strike-through 31 34 "s") " " (code 35 43 "c~d ~c") ". " (bold 45 51 "a *b") " c*""#,
        );
    }

    #[test]
    fn emphasis_borders() {
        check(
            r#"x *a* _b_ +c+ =d= ~e~ a*b* *a*b *a,*. (*a*) {*a*} -*a*- *a*: *a*\ *a*["#,
            r#""x " (bold 2 5 "a") " " (underline 6 9 "b") " " (strike-through 10 13 "c") " " (verbatim 14 17 "d") " " (code 18 21 "e") " a*b* " (bold 27 36 "a*b *a,") ". (" (bold 39 42 "a") ") {" (bold 45 48 "a") "} -" (bold 51 54 "a") "- " (bold 56 59 "a") ": " (bold 61 64 "a") "\\ " (bold 66 69 "a") "[""#,
        );
    }

    #[test]
    fn emphasis_multiline() {
        check(
            r#"*a
b
c* x \\  
y"#,
            r#"(bold 0 7 "a\nb\nc") " x " (line-break 10 15) "y""#,
        );
    }

    #[test]
    fn emphasis_nested() {
        check(
            r#"*/a/* _/*b*/_ =*a*= ~/x/~ *a=b*c=*"#,
            r#"(bold 0 5 (italic 1 4 "a")) " " (underline 6 13 (italic 7 12 (bold 8 11 "b"))) " " (verbatim 14 19 "*a*") " " (code 20 25 "/x/") " " (bold 26 34 "a=b*c=")"#,
        );
    }

    #[test]
    fn plain_link_case() {
        check(
            r#"HTTPS://x.com Https://y.org/a"#,
            r#"(link 0 13 HTTPS "//x.com" plain - -) " " (link 14 29 Https "//y.org/a" plain - -)"#,
        );
    }

    #[test]
    fn plain_link_word_boundary() {
        check(
            r#"'https://x.com a-https://y.com"#,
            r#""'https://x.com a-" (link 17 30 https "//y.com" plain - -)"#,
        );
    }

    #[test]
    fn entities() {
        check(
            r#"\\\\alpha \alpha \alphax \alpha{}x \_  a \frac12 \there4 \sup1x \Alpha{} \_"#,
            r#""\\\\\\" (entity 3 9 alpha nil) " " (entity 10 16 alpha nil) " " (latex-fragment 17 24 "\\alphax") " " (entity 25 33 alpha t) "x " (entity 35 39 _   nil) "a " (entity 41 48 frac12 nil) " " (entity 49 56 there4 nil) " " (entity 57 61 sup nil) "1x " (entity 64 72 Alpha t) " \\_""#,
        );
    }

    #[test]
    fn subsup_forms() {
        check(
            r#"a_(b) x^{a{b}{c{d}}} y^{a{b}} z_{{{x}}} z_{{{{x}}}} a^* a_-1.2 a_1."#,
            r#""a" (subscript 1 5 nil "(b)") " x^{a{b}{c{d}}} y" (superscript 22 29 t "a{b}") " z" (subscript 31 39 t "{{x}}") " z_{" (macro 43 50 x) "} a" (superscript 53 55 nil "*") " a" (subscript 57 62 nil "-1.2") " a" (subscript 64 66 nil "1") ".""#,
        );
    }

    #[test]
    fn latex_fragments() {
        check(
            r#"$a$ $ a$ a$b$ $a$b $a$, $$x y$$ \(x\) \[y\] \foo[a]{b}{c} \foo*{x}"#,
            r#"(latex-fragment 0 3 "$a$") " $ a$ a" (latex-fragment 10 13 "$b$") " $a$b " (latex-fragment 19 22 "$a$") ", " (latex-fragment 24 31 "$$x y$$") " " (latex-fragment 32 37 "\\(x\\)") " " (latex-fragment 38 43 "\\[y\\]") " " (latex-fragment 44 57 "\\foo[a]{b}{c}") " " (latex-fragment 58 66 "\\foo*{x}")"#,
        );
    }

    #[test]
    fn timestamps_ranges() {
        check(
            r#"<2024-01-01 Mon>--<2024-01-02 Tue> [2024-01-01]--[2024-01-02 10:00] <2024-01-01 Mon 9:00> <%%(diary-float t 4 2)> <%%(foo) 10:00-11:30>"#,
            r#"(timestamp 0 34 active-range "<2024-01-01 Mon>--<2024-01-02 Tue>") " " (timestamp 35 67 inactive-range "[2024-01-01]--[2024-01-02 10:00]") " " (timestamp 68 89 active "<2024-01-01 Mon 9:00>") " " (timestamp 90 113 diary "<%%(diary-float t 4 2)>") " " (timestamp 114 135 diary "<%%(foo) 10:00-11:30>")"#,
        );
    }

    #[test]
    fn bracket_link_types() {
        check(
            r#"[[file:a.org::*h][d]] [[./x.png]] [[/abs]] [[#cid]] [[(ref)]] [[*Head line]] [[foo
  bar]] [[a\]b]] [[x:y]] [[id:123]] [[file+emacs:f.org::12]] [[file:///tmp/x]]"#,
            r#"(link 0 21 file "a.org" bracket "*h" - "d") " " (link 22 33 file "./x.png" bracket - -) " " (link 34 42 file "/abs" bracket - -) " " (link 43 51 custom-id "cid" bracket - -) " " (link 52 61 coderef "ref" bracket - -) " " (link 62 76 fuzzy "*Head line" bracket - -) " " (link 77 90 fuzzy "foo bar" bracket - -) " " (link 91 99 fuzzy "a]b" bracket - -) " " (link 100 107 fuzzy "x:y" bracket - -) " " (link 108 118 id "123" bracket - -) " " (link 119 143 file "f.org" bracket "12" "emacs") " " (link 144 161 file "/tmp/x" bracket - -)"#,
        );
    }

    #[test]
    fn angle_and_plain_links() {
        check(
            r#"<https://a.b/c d> <mailto:x@y> https://a.com/b(c)d. http://x.org/a, file:foo.org::bar shell:ls see:x"#,
            r#"(link 0 17 https "//a.b/c d" angle - -) " " (link 18 30 mailto "x@y" angle - -) " " (link 31 50 https "//a.com/b(c)d" plain - -) ". " (link 52 66 http "//x.org/a" plain - -) ", " (link 68 85 file "foo.org" plain "bar" -) " " (link 86 94 shell "ls" plain - -) " see:x""#,
        );
    }

    #[test]
    fn footnotes_and_cookies() {
        check(
            r#"x [fn:1] [fn::def *b*] [fn:x:y [a] z] [fn:a [33%] [1/3] [/] [%] [2/]"#,
            r#""x " (footnote-reference 2 8 "1" standard) " " (footnote-reference 9 22 - inline "def " (bold 18 21 "b")) " " (footnote-reference 23 37 "x" inline "y [a] z") " [fn:a " (statistics-cookie 44 49 "[33%]") " " (statistics-cookie 50 55 "[1/3]") " " (statistics-cookie 56 59 "[/]") " " (statistics-cookie 60 63 "[%]") " " (statistics-cookie 64 68 "[2/]")"#,
        );
    }

    #[test]
    fn snippets_babel_src_macro_targets() {
        check(
            r#"src_python{1+{2}} src_sh[:x 1]{echo} call_f(a=1) call_g[:r]( x )[:e] {{{Mac(a\,b, c)}}} {{{m}}} @@html:<b>@@ <<t>> << t>> <<<r>>>"#,
            r#"(inline-src-block 0 17 python - "1+{2}") " " (inline-src-block 18 36 sh ":x 1" "echo") " " (inline-babel-call 37 48 f - "a=1" -) " " (inline-babel-call 49 68 g ":r" " x " ":e") " " (macro 69 87 mac "a,b" " c") " " (macro 88 95 m) " " (export-snippet 96 108 html "<b>") " " (target 109 114 "t") " << t>> " (radio-target 122 129 "r")"#,
        );
    }

    #[test]
    fn citations() {
        check(
            r#"[cite/s/x:pre;@a s1; pp @b s2;suf] [cite:@a;x;y] [cite:;@a;] [cite:pre @a] [cite:@a suf]"#,
            r#"(citation 0 34 "s/x" "pre" "suf" (citation-reference 14 20 a - " s1") (citation-reference 20 30 b " pp " " s2")) " " (citation 35 48 - - "y" (citation-reference 41 44 a - -) "x;") " " (citation 49 60 - - - (citation-reference 56 59 a - -)) " " (citation 61 74 - - - (citation-reference 67 73 a "pre " -)) " " (citation 75 88 - - - (citation-reference 81 87 a - " suf"))"#,
        );
    }

    #[test]
    fn radio_links() {
        check(
            r#"<<<My Target>>> foo my  target, xmy target MY
TARGET."#,
            r#"(radio-target 0 15 "My Target") " foo " (link 20 30 radio "my  target" plain - - "my  target") ", xmy target " (link 43 52 radio "MY\nTARGET" plain - - "MY\nTARGET") ".""#,
        );
    }

    #[test]
    fn link_abbreviations() {
        check(
            r#"#+LINK: gh https://github.com/%s
#+LINK: wp https://en.wikipedia.org/wiki/%h
#+LINK: ex https://ex.com/
[[gh:foo/bar]] [[wp:A B]] [[ex:z]] [[ex]] gh:x"#,
            r#"(link 104 118 https "//github.com/foo/bar" bracket - -) " " (link 119 129 https "//en.wikipedia.org/wiki/A%20B" bracket - -) " " (link 130 138 https "//ex.com/z" bracket - -) " " (link 139 145 https "//ex.com/" bracket - -) " gh:x""#,
        );
    }

    #[test]
    fn macro_arguments() {
        check(
            r#"{{{m()}}} {{{m( a , b\\,c )}}} {{{m(x)}}}} {{{m(a)b)}}}"#,
            r#"(macro 0 9 m "") " " (macro 10 30 m "a " " b\\" "c") " " (macro 31 41 m "x") "} " (macro 43 55 m "a)b")"#,
        );
    }

    #[test]
    fn timestamps_misc() {
        check(
            r#"<2024-01-01]  [2024-01-01 Mon 10:00]--<2024-01-03> <2024-01-01 Mon 10:00>--<2024-01-02> <2024-01-01 Mon ++1w/2d --3d> <2024-01-01 .+2m> [2024-01-01 Mon 25:61] <%%(a (b)) c) 9:15>"#,
            r#"(timestamp 0 12 active "<2024-01-01]") "  " (timestamp 14 50 inactive-range "[2024-01-01 Mon 10:00]--<2024-01-03>") " " (timestamp 51 87 active-range "<2024-01-01 Mon 10:00>--<2024-01-02>") " " (timestamp 88 117 active "<2024-01-01 Mon ++1w/2d --3d>") " " (timestamp 118 135 active "<2024-01-01 .+2m>") " " (timestamp 136 158 inactive "[2024-01-01 Mon 25:61]") " " (timestamp 159 178 diary "<%%(a (b)) c) 9:15>")"#,
        );
    }

    #[test]
    fn links_mixed() {
        check(
            r#"x 'https://a.org (https://b.org/x) <http://c.d/e
  f> mailto:a@b.c [[http://x.org][desc *b* [[http://y]] _u_ https://z.org]] [[file:a.org][img]]"#,
            r#""x 'https://a.org (" (link 18 33 https "//b.org/x" plain - -) ") " (link 35 53 http "//c.d/ef" angle - -) " " (link 54 66 mailto "a@b.c" plain - -) " " (link 67 104 http "//x.org" bracket - - "desc " (bold 88 91 "b") " [[http://y") " " (underline 105 108 "u") " " (link 109 122 https "//z.org" plain - -) "]] " (link 125 144 file "a.org" bracket - - "img")"#,
        );
    }

    #[test]
    fn bracket_link_escapes() {
        check(
            r#"[[a][b] [[c]] [[a\\\]] x"#,
            r#"(link 0 13 fuzzy "a" bracket - - "b] [[c") " " (link 14 22 fuzzy "a\\" bracket - -) " x""#,
        );
    }

    #[test]
    fn line_breaks() {
        check(
            r#"a\\
\\b \\  c \\"#,
            r#""a" (line-break 1 4) "\\" (latex-fragment 5 7 "\\b") " \\\\  c " (line-break 14 16)"#,
        );
    }

    #[test]
    fn babel_src_case() {
        check(
            r#"call_f[x(1)](a=(1)) src_c[]{} src_ x CALL_f() Src_f{} xsrc_c{a} a_src_x{1}"#,
            r#"(inline-babel-call 0 19 f "x(1)" "a=(1)" -) " " (inline-src-block 20 29 c - "") " src_ x CALL" (subscript 41 43 nil "f") "() Src" (subscript 49 51 nil "f") "{} xsrc" (subscript 58 60 nil "c") "{a} a" (subscript 65 69 nil "src") (subscript 69 71 nil "x") "{1}""#,
        );
    }

    #[test]
    fn latex_dollar() {
        check(
            r#"$x$y $x$- $x$' $x$" $x$( $x$$ $1 and $2$ x$$ x $$a
b$$ $a
b$"#,
            r#""$x$y $x$- " (latex-fragment 10 13 "$x$") "' " (latex-fragment 15 18 "$x$") "\" " (latex-fragment 20 23 "$x$") "( $x" (latex-fragment 27 44 "$$ $1 and $2$ x$$") " x " (latex-fragment 47 54 "$$a\nb$$") " " (latex-fragment 55 60 "$a\nb$")"#,
        );
    }

    #[test]
    fn timestamps_time_range() {
        check(
            r#"a <2024-01-01 Mon 10:00-11:00> b [2024-01-01 Mon 10:00-11:00]--[2024-01-02] <2024-01-01 Mon>--<2024-01-02 Tue 3:00>"#,
            r#""a " (timestamp 2 30 active-range "<2024-01-01 Mon 10:00-11:00>") " b " (timestamp 33 75 inactive-range "[2024-01-01 Mon 10:00-11:00]--[2024-01-02]") " " (timestamp 76 115 active-range "<2024-01-01 Mon>--<2024-01-02 Tue 3:00>")"#,
        );
    }

    #[test]
    fn emphasis_deep_nesting() {
        check(
            r#"*bold /italic _under +strike+ under_ italic/ bold* =a *b* c= ~a /b/~"#,
            r#"(bold 0 50 "bold " (italic 6 44 "italic " (underline 14 36 "under " (strike-through 21 29 "strike") " under") " italic") " bold") " " (verbatim 51 60 "a *b* c") " " (code 61 68 "a /b/")"#,
        );
    }

    #[test]
    fn subsup_nesting() {
        check(
            r#"x_{a_{b}} x^{*b*} x_{} x^() x_a_b x__a a^-b x^+ x_. H_2O^2"#,
            r#""x" (subscript 1 9 t "a" (subscript 4 8 t "b")) " x" (superscript 11 17 t (bold 13 16 "b")) " x" (subscript 19 22 t) " x" (superscript 24 27 nil "()") " x" (subscript 29 31 nil "a") (subscript 31 33 nil "b") " x_" (subscript 36 38 nil "a") " a" (superscript 40 43 nil "-b") " x^+ x_. H" (subscript 53 56 nil "2O") (superscript 56 58 nil "2")"#,
        );
    }

    #[test]
    fn unicode() {
        check(
            r#"αβ_γ *é* /ü/ \alpha{}é x_é ü^ü «*a*» 中*文*字 x"#,
            r#""αβ" (subscript 2 4 nil "γ") " " (bold 5 8 "é") " " (italic 9 12 "ü") " " (entity 13 21 alpha t) "é x" (subscript 24 26 nil "é") " ü" (superscript 28 30 nil "ü") " «*a*» 中*文*字 x""#,
        );
    }

    #[test]
    fn link_description_objects() {
        check(
            r#"[[https://example.com/a?b=c&d#e][Example *bold* \alpha [33%] {{{m}}} src_x{y} call_z() @@h:x@@ =v= ~c~ https://not.link <<t>>]]"#,
            r#"(link 0 127 https "//example.com/a?b=c&d#e" bracket - - "Example " (bold 41 47 "bold") " " (entity 48 54 alpha nil) " " (statistics-cookie 55 60 "[33%]") " " (macro 61 68 m) " " (inline-src-block 69 77 x - "y") " " (inline-babel-call 78 86 z - - -) " " (export-snippet 87 94 h "x") " " (verbatim 95 98 "v") " " (code 99 102 "c") " https://not.link <<t>>")"#,
        );
    }

    #[test]
    fn bracket_link_edge() {
        check(
            r#"x [[]] [[ ]] [[a]b]] [[a][]] [[a][b]]]"#,
            r#""x [[]] " (link 7 12 fuzzy " " bracket - -) " [[a]b]] " (link 21 37 fuzzy "a" bracket - - "]] [[a][b") "]""#,
        );
    }

    #[test]
    fn radio_boundaries() {
        check(
            r#"<<<t>>> t te tt T. (t) t_x x-t"#,
            r#"(radio-target 0 7 "t") " " (link 8 9 radio "t" plain - - "t") " te tt " (link 16 17 radio "T" plain - - "T") ". (" (link 20 21 radio "t" plain - - "t") ") " (link 23 24 radio "t" plain - - "t") (subscript 24 26 nil "x") " x-" (link 29 30 radio "t" plain - - "t")"#,
        );
    }

    #[test]
    fn citations_edge() {
        check(
            r#"x [cite:@a] [cite/a/b:@x;@y] [cite:@a@b;@c d] [cite: ; ] [cite:no key] [CITE:@x] [cite:@a [b] c]"#,
            r#""x " (citation 2 11 - - - (citation-reference 8 10 a - -)) " " (citation 12 28 "a/b" - - (citation-reference 22 25 x - -) (citation-reference 25 27 y - -)) " " (citation 29 45 - - - (citation-reference 35 40 a@b - -) (citation-reference 40 44 c - " d")) " [cite: ; ] [cite:no key] [CITE:@x] " (citation 81 96 - - - (citation-reference 87 95 a - " [b] c"))"#,
        );
    }

    #[test]
    fn footnotes_edge() {
        check(
            r#"x [fN:1] [FN:2] [fn:a-b_c] [fn:é] [fn:1:[x]] [fn:: a [fn:2] b]"#,
            r#""x " (footnote-reference 2 8 "1" standard) " [FN:2] " (footnote-reference 16 26 "a-b_c" standard) " " (footnote-reference 27 33 "é" standard) " " (footnote-reference 34 44 "1" inline "[x]") " " (footnote-reference 45 62 - inline " a " (footnote-reference 53 59 "2" standard) " b")"#,
        );
    }

    #[test]
    fn targets() {
        check(
            r#"<<a>> <<a b>> <<a >> <<<a b>>> <<<x>> <<x>>> <<<*b* c>>> << >>"#,
            r#"(target 0 5 "a") " " (target 6 13 "a b") " <<a >> " (radio-target 21 30 "a b") " <" (target 32 37 "x") " " (target 38 43 "x") "> " (radio-target 45 56 (bold 48 51 "b") " c") " << >>""#,
        );
    }

    #[test]
    fn export_snippets() {
        check(
            r#"@@latex:\alpha@@ @@:x@@ @@a-b:x @ y@@ @@x:@@"#,
            r#"(export-snippet 0 16 latex "\\alpha") " @@:x@@ " (export-snippet 24 37 a-b "x @ y") " " (export-snippet 38 44 x "")"#,
        );
    }

    #[test]
    fn radio_bol() {
        check(
            r#"<<<a>>> a. ba a_b a
a"#,
            r#"(radio-target 0 7 "a") " " (link 8 9 radio "a" plain - - "a") ". ba " (link 14 15 radio "a" plain - - "a") (subscript 15 17 nil "b") " " (link 18 19 radio "a" plain - - "a") "\n" (link 20 21 radio "a" plain - - "a")"#,
        );
    }

    #[test]
    fn radio_spaces_case() {
        check(
            r#"<<<foo bar>>> x foo bar *foo bar* [[foo bar]] Foo	Bar foo
bar."#,
            r#"(radio-target 0 13 "foo bar") " x " (link 16 23 radio "foo bar" plain - - "foo bar") " " (bold 24 33 (link 25 32 radio "foo bar" plain - - "foo bar")) " " (link 34 45 fuzzy "foo bar" bracket - -) " " (link 46 53 radio "Foo	Bar" plain - - "Foo	Bar") " " (link 54 61 radio "foo\nbar" plain - - "foo\nbar") ".""#,
        );
    }

    #[test]
    fn radio_inside_markup() {
        check(
            r#"<<<b>>> *a b c* b*b"#,
            r#"(radio-target 0 7 "b") " " (bold 8 15 "a " (link 11 12 radio "b" plain - - "b") " c") " " (link 16 17 radio "b" plain - - "b") "*" (link 18 19 radio "b" plain - - "b")"#,
        );
    }

    #[test]
    fn radio_longest() {
        check(
            r#"<<<x y>>> <<<x>>> x y x"#,
            r#"(radio-target 0 9 "x y") " " (radio-target 10 17 "x") " " (link 18 21 radio "x y" plain - - "x y") " " (link 22 23 radio "x" plain - - "x")"#,
        );
    }

    #[test]
    fn plain_link_paths() {
        check(
            r#"https://a.org/(x(y)z) https://a.org/x(y http://a.b/c.d/ http://a.b/c- http://a.b/c_ http://a.b/c! http://a.b/[x] http://a http://ab <http://x.org> <file:a b
c> <https://x
>"#,
            r#"(link 0 21 https "//a.org/(x(y)z)" plain - -) " " (link 22 37 https "//a.org/x" plain - -) "(y " (link 40 55 http "//a.b/c.d/" plain - -) " " (link 56 69 http "//a.b/c-" plain - -) " " (link 70 82 http "//a.b/c" plain - -) "_ " (link 84 96 http "//a.b/c" plain - -) "! " (link 98 112 http "//a.b/[x]" plain - -) " " (link 113 121 http "//a" plain - -) " " (link 122 131 http "//ab" plain - -) " " (link 132 146 http "//x.org" angle - -) " " (link 147 159 file "a bc" angle - -) " <" (link 161 170 https "//x" plain - -) "\n>""#,
        );
    }

    #[test]
    fn plain_link_types() {
        check(
            r#"file+sys:/x/y::z file+emacs:a FILE:x.org doi:10.1/2 id:abc-1 elisp:(foo) help:x info:org#Links news:x irc:y bbdb:z mhe:q"#,
            r#"(link 0 16 file "/x/y" plain "z" "sys") " file+emacs:a " (link 30 40 file "x.org" plain - -) " " (link 41 51 doi "10.1/2" plain - -) " " (link 52 60 id "abc-1" plain - -) " elisp:(foo) help:x " (link 80 94 info "org#Links" plain - -) " news:x irc:y bbdb:z mhe:q""#,
        );
    }

    #[test]
    fn bracket_link_files() {
        check(
            r#"[[file:/x::y::z]] [[file:a b.org]] [[~/x]] [[../a]] [[FILE:x]] [[https:x][a]] [[docview:a.pdf::3]] [[file:a.org::#cid]] [[file:////tmp]] [[file://C:/x]]"#,
            r##"(link 0 17 file "/x" bracket "y::z" -) " " (link 18 34 file "a b.org" bracket - -) " " (link 35 42 file "~/x" bracket - -) " " (link 43 51 file "../a" bracket - -) " " (link 52 62 file "x" bracket - -) " " (link 63 77 https "x" bracket - - "a") " " (link 78 98 docview "a.pdf::3" bracket - -) " " (link 99 119 file "a.org" bracket "#cid" -) " " (link 120 136 file "/tmp" bracket - -) " " (link 137 152 file "C:/x" bracket - -)"##,
        );
    }

    #[test]
    fn citation_prefix_forms() {
        check(
            r#"[cITE:@x] [cite/:@x] [cite/a b:@x] [cite:@x
] [cite:
@x;y]"#,
            r#"(citation 0 9 - - - (citation-reference 6 8 x - -)) " [cite/:@x] [cite/a b:@x] " (citation 35 45 - - - (citation-reference 41 43 x - -)) " " (citation 46 58 - - "y" (citation-reference 53 56 x - -))"#,
        );
    }

    #[test]
    fn latex_edge() {
        check(
            r#"$$ $$$ $a$$ $$a$ $x$y$ \(a \) b \[ \] \(a
b\) \begin{x} \x{a}{b}[c] \x [a] \x{a
b} \x*"#,
            r#"(latex-fragment 0 5 "$$ $$") "$ $a" (latex-fragment 9 14 "$$ $$") "a$ $x" (latex-fragment 19 22 "$y$") " " (latex-fragment 23 29 "\\(a \\)") " b " (latex-fragment 32 37 "\\[ \\]") " " (latex-fragment 38 45 "\\(a\nb\\)") " " (latex-fragment 46 55 "\\begin{x}") " " (latex-fragment 56 67 "\\x{a}{b}[c]") " " (latex-fragment 68 70 "\\x") " [a] " (latex-fragment 75 77 "\\x") "{a\nb} " (latex-fragment 83 86 "\\x*")"#,
        );
    }

    #[test]
    fn entities_edge() {
        check(
            r#"\alpha\beta \alpha_{x} \pi^2 \nbsp{}x \S \textbackslash \ldots \- \_    \_   x \dots{}"#,
            r#"(entity 0 6 alpha nil) (entity 6 11 beta nil) " " (entity 12 18 alpha nil) (subscript 18 22 t "x") " " (entity 23 26 pi nil) (superscript 26 28 nil "2") " " (entity 29 36 nbsp t) "x " (entity 38 40 S nil) " " (latex-fragment 41 55 "\\textbackslash") " " (latex-fragment 56 62 "\\ldots") " \\- " (entity 66 72 _     nil) (entity 72 77 _    nil) "x " (entity 79 86 dots t)"#,
        );
    }

    #[test]
    fn statistics_cookies() {
        check(
            r#"[1/2] [1%] [10/] [/10] [1/2%] [12%%] [%] [/]"#,
            r#"(statistics-cookie 0 5 "[1/2]") " " (statistics-cookie 6 10 "[1%]") " " (statistics-cookie 11 16 "[10/]") " " (statistics-cookie 17 22 "[/10]") " [1/2%] [12%%] " (statistics-cookie 37 40 "[%]") " " (statistics-cookie 41 44 "[/]")"#,
        );
    }

    #[test]
    fn timestamps_invalid() {
        check(
            r#"<2024-01-01 Mon 10:00>--<2024-01-02 Tue 11:00> <2024-1-01> <2024-01-01x> <2024-01-01
> <%%(foo]bar)> [2024-01-01 Mon]x <2024-01-01 Mon 10:00-11:00 +2d -3d> <2024-01-01 +1d/3d>"#,
            r#"(timestamp 0 46 active-range "<2024-01-01 Mon 10:00>--<2024-01-02 Tue 11:00>") " <2024-1-01> <2024-01-01x> <2024-01-01\n> " (timestamp 87 95 diary "<%%(foo]") "bar)> " (timestamp 101 117 inactive "[2024-01-01 Mon]") "x " (timestamp 119 155 active-range "<2024-01-01 Mon 10:00-11:00 +2d -3d>") " " (timestamp 156 175 active "<2024-01-01 +1d/3d>")"#,
        );
    }

    #[test]
    fn macros_edge() {
        check(
            r#"{{{key-name_2(a,b,c)}}} {{{2x}}} {{{x(a
b)}}} {{{x(\,a\\\,b)}}} {{{X(a,,b)}}}"#,
            r#"(macro 0 23 key-name_2 "a" "b" "c") " {{{2x}}} " (macro 33 45 x "a b") " " (macro 46 63 x ",a\\,b") " " (macro 64 77 x "a" "" "b")"#,
        );
    }

    #[test]
    fn line_breaks_edge() {
        check(
            r#"a \\ b \\
c\\\\
d \\	
e \\x"#,
            r#""a \\\\ b " (line-break 7 10) "c\\\\\\\\\nd " (line-break 18 22) "e \\" (latex-fragment 25 27 "\\x")"#,
        );
    }

    #[test]
    fn emphasis_post_chars() {
        check(
            r#"*a* b *c*d *e*. /f/: _g_; +h+! ~i~? =j=' *k*" *l*) *m*} *n*\ *o*["#,
            r#"(bold 0 3 "a") " b " (bold 6 14 "c*d *e") ". " (italic 16 19 "f") ": " (underline 21 24 "g") "; " (strike-through 26 29 "h") "! " (code 31 34 "i") "? " (verbatim 36 39 "j") "' " (bold 41 44 "k") "\" " (bold 46 49 "l") ") " (bold 51 54 "m") "} " (bold 56 59 "n") "\\ " (bold 61 64 "o") "[""#,
        );
    }

    #[test]
    fn emphasis_pre_chars() {
        check(
            r#"x*a* x/a/ x_a_ 1+a+ "*a*" '*a*' {*a*} (*a*) -*a*- :*a*: ,*a*, ;*a*;"#,
            r#""x*a* x/a/ x" (subscript 11 13 nil "a") "_ 1+a+ \"" (bold 21 24 "a") "\" '" (bold 27 30 "a") "' {" (bold 33 36 "a") "} (" (bold 39 42 "a") ") -" (bold 45 48 "a") "- :*a*: ,*a*, ;*a*;""#,
        );
    }

    #[test]
    fn underline_vs_subscript() {
        check(
            r#"_a _b_ c_ d_e_f_ __a__ ___ _ _"#,
            r#"(underline 0 6 "a _b") " c_ d" (subscript 11 13 nil "e") (subscript 13 15 nil "f") "_ " (underline 17 22 (underline 18 21 "a")) " " (underline 23 26 "_") " _ _""#,
        );
    }

    #[test]
    fn subsup_chains() {
        check(
            r#"a^{b}c^d^e x^b_c x_(a(b)) x_((a)) x_(((a))) x_((((a)))) x_{a}{b}"#,
            r#""a" (superscript 1 5 t "b") "c" (superscript 6 8 nil "d") (superscript 8 10 nil "e") " x" (superscript 12 14 nil "b") (subscript 14 16 nil "c") " x" (subscript 18 25 nil "(a(b))") " x" (subscript 27 33 nil "((a))") " x" (subscript 35 43 nil "(((a)))") " x_((((a)))) x" (subscript 57 61 t "a") "{b}""#,
        );
    }

    #[test]
    fn subscript_bodies() {
        check(
            r#"foo_bar_baz a_1,2 a_1, a_\alpha a_. a_+ a_-x"#,
            r#""foo" (subscript 3 7 nil "bar") (subscript 7 11 nil "baz") " a" (subscript 13 17 nil "1,2") " a" (subscript 19 21 nil "1") ", a" (subscript 24 31 nil (entity 25 31 alpha nil)) " a_. a_+ a" (subscript 41 44 nil "-x")"#,
        );
    }

    #[test]
    fn inline_src_blocks() {
        check(
            r#"src_emacs-lisp[:exports both]{(+ 1 2)} src_c{int a{}; } src_x[a
 b]{y} src_x[ ]{} src_x{a"#,
            r#"(inline-src-block 0 38 emacs-lisp ":exports both" "(+ 1 2)") " " (inline-src-block 39 55 c - "int a{}; ") " " (inline-src-block 56 70 x "a b" "y") " " (inline-src-block 71 81 x - "") " src" (subscript 85 87 nil "x") "{a""#,
        );
    }

    #[test]
    fn inline_babel_calls() {
        check(
            r#"call_f() call_f[] call_f[](x) call_f(x)[] call_f(x)[:a
 b] call_f(a)(b) call_ f(x)"#,
            r#"(inline-babel-call 0 8 f - - -) " call" (subscript 13 15 nil "f") "[] " (inline-babel-call 18 29 f - "x" -) " " (inline-babel-call 30 41 f - "x" -) " " (inline-babel-call 42 58 f - "x" ":a b") " " (inline-babel-call 59 68 f - "a" -) "(b) call_ f(x)""#,
        );
    }

    #[test]
    fn export_snippets_edge() {
        check(
            r#"@@html:a@@b@@ @@html:@@@@ @@ht ml:x@@ @@html:a
b@@"#,
            r#"(export-snippet 0 10 html "a") "b@@ " (export-snippet 14 23 html "") "@@ @@ht ml:x@@ " (export-snippet 38 50 html "a\nb")"#,
        );
    }

    #[test]
    fn bracket_link_descriptions() {
        check(
            r#"x [[a]] [[a][b [[c][d]] e]] [[*a][*b*]] [[#x][=y=]] [[a][b\]c]]"#,
            r#""x " (link 2 7 fuzzy "a" bracket - -) " " (link 8 23 fuzzy "a" bracket - - "b [[c][d") " e]] " (link 28 39 fuzzy "*a" bracket - - (bold 34 37 "b")) " " (link 40 51 custom-id "x" bracket - - (verbatim 46 49 "y")) " " (link 52 63 fuzzy "a" bracket - - "b\\]c")"#,
        );
    }

    #[test]
    fn link_abbreviations_edge() {
        check(
            r#"#+LINK: a http://a/%s/%s
#+LINK: b http://b/
#+LINK: c %(foo)
#+LINK: d http://d/%h?q=%s
[[a:x]] [[a::y]] [[b:é ü]] [[c:x]] [[d:a b]] [[a]] a:x [[b]]
"#,
            r#"(link 89 96 http "//a/x/%s" bracket - -) " " (link 97 105 http "//a/y/%s" bracket - -) " " (link 106 115 http "//b/é ü" bracket - -) " " (link 116 123 fuzzy "c:x" bracket - -) " " (link 124 133 http "//d/%h?q=a b" bracket - -) " " (link 134 139 http "//a//%s" bracket - -) " a:x " (link 144 149 http "//b/" bracket - -) "\n""#,
        );
    }

    #[test]
    fn table_cell_restriction() {
        check_element(
            r#"| a *b* | [[x][y]] | call_f() | src_a{b} | [1/2] | x\\ | <<t>> | <<<r>>> r | <2024-01-01> | [fn:1] | [cite:@a] | {{{m}}} | @@h:x@@ | \alpha | $x$ | a_b |"#,
            r#""a " (bold 4 7 "b") | (link 10 18 fuzzy "x" bracket - - "y") | "call" (subscript 25 27 nil "f") "()" | "src" (subscript 35 37 nil "a") "{b}" | "[1/2]" | "x\\\\" | (target 57 62 "t") | (radio-target 65 72 "r") " " (link 73 74 radio "r" plain - - "r") | (timestamp 77 89 active "<2024-01-01>") | (footnote-reference 92 98 "1" standard) | (citation 101 110 - - - (citation-reference 107 109 a - -)) | (macro 113 120 m) | (export-snippet 123 130 h "x") | (entity 133 139 alpha nil) | (latex-fragment 142 145 "$x$") | "a" (subscript 149 151 nil "b")"#,
        );
    }

    #[test]
    fn headline_title_no_line_break() {
        check_element(r#"* a \\"#, r#""a \\\\""#);
    }

    #[test]
    fn item_tag_restriction() {
        check_element(
            r#"- *t* \\ [[x]] [fn:1] :: body"#,
            r#"(bold 2 5 "t") " \\\\ " (link 9 14 fuzzy "x" bracket - -) " " (footnote-reference 15 21 "1" standard)"#,
        );
    }

    #[test]
    fn link_description_restriction() {
        check(
            r#"x [[a][[[b]]]] [[a][<<t>> <<<r>>> x\\ [fn:1] [cite:@a] <2024-01-01>]]"#,
            r#""x " (link 2 12 fuzzy "a" bracket - - "[[b") "]] " (link 15 69 fuzzy "a" bracket - - "<<t>> <<<r>>> x\\\\ [fn:1] [cite:@a] <2024-01-01>")"#,
        );
    }

    #[test]
    fn emphasis_over_link() {
        check(
            r#"*a [[b][c*]] d*"#,
            r#"(bold 0 15 "a " (link 3 12 fuzzy "b" bracket - - "c*") " d")"#,
        );
    }

    #[test]
    fn subsup_contents() {
        check(
            r#"x_{[[a]]} x^{a\\} [fn::a \\ b]
"#,
            r#""x" (subscript 1 9 t (link 3 8 fuzzy "a" bracket - -)) " x" (superscript 11 17 t "a" (line-break 14 16)) " " (footnote-reference 18 30 - inline "a \\\\ b") "\n""#,
        );
    }

    #[test]
    fn radio_inside_objects() {
        check(
            r#"<<<ab>>> x*ab* *ab* /x ab/ ab_c ab^2 \alpha ab [[ab]] ab$x$ $ab$ https://ab.org/ab ab"#,
            r#"(radio-target 0 8 "ab") " x*" (link 11 13 radio "ab" plain - - "ab") "* " (bold 15 19 (link 16 18 radio "ab" plain - - "ab")) " " (italic 20 26 "x " (link 23 25 radio "ab" plain - - "ab")) " " (link 27 29 radio "ab" plain - - "ab") (subscript 29 31 nil "c") " " (link 32 34 radio "ab" plain - - "ab") (superscript 34 36 nil "2") " " (entity 37 43 alpha nil) " " (link 44 46 radio "ab" plain - - "ab") " " (link 47 53 fuzzy "ab" bracket - -) " " (link 54 56 radio "ab" plain - - "ab") (latex-fragment 56 59 "$x$") " " (latex-fragment 60 64 "$ab$") " " (link 65 82 https "//ab.org/ab" plain - -) " " (link 83 85 radio "ab" plain - - "ab")"#,
        );
    }

    #[test]
    fn radio_line_start() {
        check(
            r#"<<<a>>> b
a b
a"#,
            r#"(radio-target 0 7 "a") " b\n" (link 10 11 radio "a" plain - - "a") " b\n" (link 14 15 radio "a" plain - - "a")"#,
        );
    }

    #[test]
    fn radio_word_chars() {
        check(
            r#"<<<foo>>> foo_bar foobar barfoo foo-bar foo.bar 中foo文 éfoo"#,
            r#"(radio-target 0 9 "foo") " " (link 10 13 radio "foo" plain - - "foo") (subscript 13 17 nil "bar") " foobar barfoo " (link 32 35 radio "foo" plain - - "foo") "-bar " (link 40 43 radio "foo" plain - - "foo") ".bar 中" (link 49 52 radio "foo" plain - - "foo") "文 éfoo""#,
        );
    }

    #[test]
    fn radio_whitespace() {
        check(
            r#"<<<x>>> <<<x y>>> x  y x
y xy"#,
            r#"(radio-target 0 7 "x") " " (radio-target 8 17 "x y") " " (link 18 22 radio "x  y" plain - - "x  y") " " (link 23 26 radio "x\ny" plain - - "x\ny") " xy""#,
        );
    }

    #[test]
    fn radio_plus() {
        check(
            r#"<<<c++>>> c++ c+ c++x"#,
            r#"(radio-target 0 9 "c++") " " (link 10 13 radio "c++" plain - - "c++") " c+ c++x""#,
        );
    }

    #[test]
    fn radio_long_first() {
        check(
            r#"<<<long target>>> <<<long>>> long target long"#,
            r#"(radio-target 0 17 "long target") " " (radio-target 18 28 "long") " " (link 29 40 radio "long target" plain - - "long target") " " (link 41 45 radio "long" plain - - "long")"#,
        );
    }

    #[test]
    fn radio_after_line_break() {
        check(
            r#"x \\
<<<t>>> t"#,
            r#""x " (line-break 2 5) (radio-target 5 12 "t") " " (link 13 14 radio "t" plain - - "t")"#,
        );
    }

    #[test]
    fn radio_in_containers() {
        check(
            r#"<<<a>>> [cite:@a a] [fn:a a] <<a>> {{{a(a)}}}"#,
            r#"(radio-target 0 7 "a") " " (citation 8 19 - - - (citation-reference 14 18 a - " a")) " [fn:" (link 24 25 radio "a" plain - - "a") " " (link 26 27 radio "a" plain - - "a") "] " (target 29 34 "a") " " (macro 35 45 a "a")"#,
        );
    }

    #[test]
    fn headline_title_markup() {
        check_element(r#"* *a* b"#, r#"(bold 2 5 "a") " b""#);
    }

    #[test]
    fn table_cell_radio() {
        check_element(
            r#"| <<<t>>> | t x | *t* |"#,
            r#"(radio-target 2 9 "t") | (link 12 13 radio "t" plain - - "t") " x" | (bold 18 21 (link 19 20 radio "t" plain - - "t"))"#,
        );
    }

    #[test]
    fn emphasis_ambiguous() {
        check(
            r#"*a*b* *a**b* *a *b* c* *x* *y* /a/b/ /a /b/ c/"#,
            r#"(bold 0 5 "a*b") " " (bold 6 12 "a**b") " " (bold 13 19 "a *b") " c* " (bold 23 26 "x") " " (bold 27 30 "y") " " (italic 31 36 "a/b") " " (italic 37 43 "a /b") " c/""#,
        );
    }

    #[test]
    fn emphasis_verbatim_ambiguous() {
        check(
            r#"a*b*c* *a\nb* a=b=c= =a= =a=b= ==a== ~~ ~a~~"#,
            r#""a*b*c* " (bold 7 13 "a" (latex-fragment 9 12 "\\nb")) " a=b=c= " (verbatim 21 24 "a") " " (verbatim 25 30 "a=b") " " (verbatim 31 36 "=a=") " " (code 37 44 "~ ~a~")"#,
        );
    }

    #[test]
    fn emphasis_quotes() {
        check(
            r#""*a*" «*a*» *a*« 'a' (a) [*a*] <*a*> |*a*| a*b*c"#,
            r#""\"" (bold 1 4 "a") "\" «*a*» *a*« 'a' (a) [*a*] <*a*> |*a*| a*b*c""#,
        );
    }

    #[test]
    fn latex_dollar_edge() {
        check(
            r#"$ $$ $$$ $$$$ \( \[ \) \] $\alpha$ \(\alpha\) $a$$b$ $a$b$ $1,000$ $a,$ $.a$ $a.$"#,
            r#""$ " (latex-fragment 2 7 "$$ $$") "$ " (latex-fragment 9 13 "$$$$") " " (latex-fragment 14 22 "\\( \\[ \\)") " \\] " (latex-fragment 26 34 "$\\alpha$") " " (latex-fragment 35 45 "\\(\\alpha\\)") " $a$$b$ $a" (latex-fragment 55 58 "$b$") " " (latex-fragment 59 66 "$1,000$") " $a,$ $.a$ $a.$""#,
        );
    }

    #[test]
    fn backslash_soup() {
        check(
            r#"\ \\ \\\ \a \1 \( \[ \_ \_  \{ \} \\\\ \frac{a}{b} \sqrt[3]{x} \x[a]"#,
            r#""\\ \\\\ \\\\\\ " (latex-fragment 9 11 "\\a") " \\1 \\( \\[ " (entity 21 24 _  nil) (entity 24 28 _   nil) "\\{ \\} \\\\\\\\ " (latex-fragment 39 50 "\\frac{a}{b}") " " (latex-fragment 51 62 "\\sqrt[3]{x}") " " (latex-fragment 63 68 "\\x[a]")"#,
        );
    }

    #[test]
    fn bracket_soup() {
        check(
            r#"[ [[ [[[ ]]] [[x] [[x]]] [[[x]]] [fn [fn: [cite [cite: [1 [% [/ [1/ [12 <2 <%% <% << <<<"#,
            r#""[ [[ [" (link 6 11 fuzzy " " bracket - -) "] [[x] " (link 18 23 fuzzy "x" bracket - -) "] [" (link 26 31 fuzzy "x" bracket - -) "] [fn [fn: [cite [cite: [1 [% [/ [1/ [12 <2 <%% <% << <<<""#,
        );
    }

    #[test]
    fn marker_soup() {
        check(
            r#"@ @@ @@@ @@@@ { {{ {{{ {{{}}} {{{a}} _ ^ * / ~ = + call_ src_ "#,
            r#""@ @@ @@@ @@@@ { {{ {{{ {{{}}} {{{a}} _ ^ * / ~ = + call_ src_ ""#,
        );
    }

    #[test]
    fn timestamps_odd_values() {
        check(
            r#"<2024-02-30> <0000-00-00> <9999-99-99 Xyz 99:99> [2024-01-01 Mon 1:2] <2024-01-01   Mon   10:00> <2024-01-01 10:00>"#,
            r#"(timestamp 0 12 active "<2024-02-30>") " " (timestamp 13 25 active "<0000-00-00>") " " (timestamp 26 48 active "<9999-99-99 Xyz 99:99>") " " (timestamp 49 69 inactive "[2024-01-01 Mon 1:2]") " " (timestamp 70 96 active "<2024-01-01   Mon   10:00>") " " (timestamp 97 115 active "<2024-01-01 10:00>")"#,
        );
    }

    #[test]
    fn timestamps_chains() {
        check(
            r#"<2024-01-01 Mon>--<2024-01-02 Tue>--<2024-01-03> <2024-01-01>-<2024-01-02> <2024-01-01>---<2024-01-02> <2024-01-01 +1w +2d -3d --4d>
"#,
            r#"(timestamp 0 34 active-range "<2024-01-01 Mon>--<2024-01-02 Tue>") "--" (timestamp 36 48 active "<2024-01-03>") " " (timestamp 49 61 active "<2024-01-01>") "-" (timestamp 62 74 active "<2024-01-02>") " " (timestamp 75 87 active "<2024-01-01>") "---" (timestamp 90 102 active "<2024-01-02>") " " (timestamp 103 132 active "<2024-01-01 +1w +2d -3d --4d>") "\n""#,
        );
    }

    #[test]
    fn script_boundary_greek() {
        check(
            r#"x αhttp://x"#,
            r#""x α" (link 3 11 http "//x" plain - -)"#,
        );
    }

    #[test]
    fn script_boundary_latin() {
        check(r#"x éhttp://x"#, r#""x éhttp://x""#);
    }

    #[test]
    fn script_boundary_cyrillic() {
        check(
            r#"x дhttp://x"#,
            r#""x д" (link 3 11 http "//x" plain - -)"#,
        );
    }

    #[test]
    fn script_boundary_han_call() {
        check(
            r#"x 中call_f()"#,
            r#""x 中" (inline-babel-call 3 11 f - - -)"#,
        );
    }

    #[test]
    fn script_boundary_kana() {
        check(
            r#"x あhttp://x"#,
            r#""x あ" (link 3 11 http "//x" plain - -)"#,
        );
    }

    #[test]
    fn script_boundary_arabic_digit() {
        check(
            r#"x ٣http://x"#,
            r#""x ٣" (link 3 11 http "//x" plain - -)"#,
        );
    }

    #[test]
    fn script_boundary_superscript_two() {
        check(r#"x ²http://x"#, r#""x ²http://x""#);
    }
}

fn events(src: &str) -> Vec<(Event<'_>, Range<usize>)> {
    let settings = snippet_settings(src);
    let mut out = Vec::new();
    parse_objects(
        src,
        paragraph_range(src),
        Restriction::Standard,
        &settings,
        &mut out,
    );
    out
}

fn first_link(src: &str) -> Link<'_> {
    events(src)
        .into_iter()
        .find_map(|(e, _)| match e {
            Event::Start(Tag::Link(l)) => Some(l),
            _ => None,
        })
        .expect("no link")
}

fn first_timestamp(src: &str) -> Timestamp<'_> {
    events(src)
        .into_iter()
        .find_map(|(e, _)| match e {
            Event::Timestamp(t) => Some(t),
            _ => None,
        })
        .expect("no timestamp")
}

fn dt(year: u16, month: u8, day: u8, hm: Option<(u8, u8)>) -> DateTime {
    DateTime {
        year,
        month,
        day,
        hour: hm.map(|x| x.0),
        minute: hm.map(|x| x.1),
    }
}

#[test]
fn smoke() {
    check(
        "a *b* [[x][y]]",
        r#""a " (bold 2 5 "b") " " (link 6 14 fuzzy "x" bracket - - "y")"#,
    );
    check("foo\nbar", r#""foo\nbar""#);
    check("", "");
}

#[test]
fn text_and_soft_breaks() {
    let src = "a\nb  \n\nc";
    let ev = events(src);
    assert_eq!(
        ev,
        vec![
            (Event::Text(CowStr::Borrowed("a")), 0..1),
            (Event::SoftBreak, 1..2),
            (Event::Text(CowStr::Borrowed("b  ")), 2..5),
            (Event::SoftBreak, 5..6),
            (Event::SoftBreak, 6..7),
            (Event::Text(CowStr::Borrowed("c")), 7..8),
        ]
    );
    for (e, _) in &ev {
        if let Event::Text(t) = e {
            assert!(matches!(t, CowStr::Borrowed(_)));
        }
    }
}

#[test]
fn object_ranges_exclude_trailing_blanks() {
    let ev = events("*a*   b");
    assert_eq!(ev[0], (Event::Start(Tag::Bold), 0..3));
    assert_eq!(ev[2], (Event::End(TagEnd::Bold), 0..3));
    assert_eq!(ev[3], (Event::Text(CowStr::Borrowed("   b")), 3..7));
}

#[test]
fn line_break_range_includes_newline() {
    let ev = events("a \\\\  \nb \\\\");
    assert_eq!(
        ev,
        vec![
            (Event::Text(CowStr::Borrowed("a ")), 0..2),
            (Event::LineBreak, 2..7),
            (Event::Text(CowStr::Borrowed("b ")), 7..9),
            (Event::LineBreak, 9..11),
        ]
    );
}

#[test]
fn absolute_ranges() {
    let src = "xxxx *a* yyyy";
    let settings = Settings::default();
    let mut out = Vec::new();
    parse_objects(src, 5..8, Restriction::Standard, &settings, &mut out);
    assert_eq!(out[0], (Event::Start(Tag::Bold), 5..8));
    assert_eq!(out[1], (Event::Text(CowStr::Borrowed("a")), 6..7));
    // Emphasis at the start of the range: the range start counts as a line
    // start (Emacs narrows to the container).
    let mut out = Vec::new();
    parse_objects("a*b*", 1..4, Restriction::Standard, &settings, &mut out);
    assert_eq!(out[0], (Event::Start(Tag::Bold), 1..4));
}

#[test]
fn link_fields() {
    let l = first_link("[[file+emacs:f.org::12][d]]");
    assert_eq!(l.link_type, "file");
    assert_eq!(l.path, "f.org");
    assert_eq!(l.raw_link, "file+emacs:f.org::12");
    assert_eq!(l.search_option.as_deref(), Some("12"));
    assert_eq!(l.application.as_deref(), Some("emacs"));
    assert_eq!(l.format, LinkFormat::Bracket);

    let l = first_link("[[a\\]b]]");
    assert_eq!(
        (&*l.link_type, &*l.path, &*l.raw_link),
        ("fuzzy", "a]b", "a\\]b")
    );

    let l = first_link("[[foo\n  bar]]");
    assert_eq!((&*l.path, &*l.raw_link), ("foo bar", "foo\n  bar"));

    let l = first_link("x <http://c.d/e\n  f> y");
    assert_eq!(
        (&*l.link_type, &*l.path, &*l.raw_link),
        ("http", "//c.d/ef", "http://c.d/e\n  f")
    );
    assert_eq!(l.format, LinkFormat::Angle);

    let l = first_link("see file:foo.org::bar.");
    assert_eq!(
        (&*l.link_type, &*l.path, &*l.raw_link),
        ("file", "foo.org", "file:foo.org::bar")
    );
    assert_eq!(l.search_option.as_deref(), Some("bar"));
    assert_eq!(l.format, LinkFormat::Plain);

    let l = first_link("[[file:///tmp/x]]");
    assert_eq!(l.path, "/tmp/x");
    let l = first_link("[[(ref)]]");
    assert_eq!((&*l.link_type, &*l.path), ("coderef", "ref"));
    let l = first_link("[[#cid]]");
    assert_eq!((&*l.link_type, &*l.path), ("custom-id", "cid"));
    let l = first_link("[[*Head]]");
    assert_eq!((&*l.link_type, &*l.path), ("fuzzy", "*Head"));
    let l = first_link("[[id:123]]");
    assert_eq!((&*l.link_type, &*l.path), ("id", "123"));
    // Paths are borrowed when nothing had to be rewritten.
    assert!(matches!(l.path, CowStr::Borrowed(_)));
}

#[test]
fn link_abbreviation_fields() {
    let src = "#+LINK: gh https://github.com/%s\n#+LINK: wp https://en.wikipedia.org/wiki/%h\n[[gh:a/b]] [[wp:A B]]";
    let ev = events(src);
    let links: Vec<_> = ev
        .iter()
        .filter_map(|(e, _)| match e {
            Event::Start(Tag::Link(l)) => Some(l.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(links[0].link_type, "https");
    assert_eq!(links[0].path, "//github.com/a/b");
    assert_eq!(links[0].raw_link, "gh:a/b");
    assert_eq!(links[1].path, "//en.wikipedia.org/wiki/A%20B");
}

#[test]
fn radio_link_fields() {
    let src = "<<<My Target>>> see my\ntarget.";
    let ev = events(src);
    let i = ev
        .iter()
        .position(|(e, _)| matches!(e, Event::Start(Tag::Link(_))))
        .unwrap();
    let Event::Start(Tag::Link(l)) = &ev[i].0 else {
        unreachable!()
    };
    assert_eq!(l.link_type, "radio");
    assert_eq!(l.path, "my\ntarget");
    assert_eq!(l.format, LinkFormat::Plain);
    assert_eq!(ev[i].1, 20..29);
    assert_eq!(ev[i + 1].0, Event::Text(CowStr::Borrowed("my")));
    assert_eq!(ev[i + 2].0, Event::SoftBreak);
    assert_eq!(ev[i + 3].0, Event::Text(CowStr::Borrowed("target")));
    assert_eq!(ev[i + 4].0, Event::End(TagEnd::Link));
}

#[test]
fn timestamp_fields() {
    let t = first_timestamp("<2024-01-02 Tue 10:00-11:00 +1w -2d>");
    assert_eq!(t.kind, TimestampKind::ActiveRange);
    assert_eq!(t.raw, "<2024-01-02 Tue 10:00-11:00 +1w -2d>");
    assert_eq!(t.start, Some(dt(2024, 1, 2, Some((10, 0)))));
    assert_eq!(t.end, Some(dt(2024, 1, 2, Some((11, 0)))));
    assert_eq!(
        t.repeater,
        Some(Repeater {
            kind: RepeaterKind::Cumulate,
            value: 1,
            unit: TimeUnit::Week
        })
    );
    assert_eq!(
        t.warning,
        Some(Warning {
            kind: WarningKind::All,
            value: 2,
            unit: TimeUnit::Day
        })
    );

    let t = first_timestamp("<2024-01-01 Mon ++1w/2d --3d>");
    assert_eq!(t.kind, TimestampKind::Active);
    assert_eq!(t.start, Some(dt(2024, 1, 1, None)));
    assert_eq!(t.end, None);
    assert_eq!(t.repeater.unwrap().kind, RepeaterKind::CatchUp);
    assert_eq!(
        t.warning,
        Some(Warning {
            kind: WarningKind::First,
            value: 3,
            unit: TimeUnit::Day
        })
    );

    let t = first_timestamp("x <2024-01-01 .+2m>");
    assert_eq!(
        t.repeater,
        Some(Repeater {
            kind: RepeaterKind::Restart,
            value: 2,
            unit: TimeUnit::Month
        })
    );

    // The end of a date range inherits the start time when it has none.
    let t = first_timestamp("[2024-01-01 Mon 10:00]--<2024-01-03>");
    assert_eq!(t.kind, TimestampKind::InactiveRange);
    assert_eq!(t.end, Some(dt(2024, 1, 3, Some((10, 0)))));

    let t = first_timestamp("[2024-01-01 Mon 9:05]");
    assert_eq!(t.kind, TimestampKind::Inactive);
    assert_eq!(t.start, Some(dt(2024, 1, 1, Some((9, 5)))));

    let t = first_timestamp("<%%(diary-float t 4 2)>");
    assert_eq!(t.kind, TimestampKind::Diary);
    assert_eq!(t.raw, "<%%(diary-float t 4 2)>");
    assert_eq!(
        (t.start, t.end, t.repeater, t.warning),
        (None, None, None, None)
    );
}

#[test]
fn entity_fields() {
    let ev = events("\\alpha{}x \\nbsp");
    let Event::Entity(e) = &ev[0].0 else {
        panic!("{ev:?}")
    };
    assert_eq!(ev[0].1, 0..8);
    assert_eq!(e.name, "alpha");
    assert_eq!(e.latex, "\\alpha");
    assert!(e.latex_math);
    assert_eq!(e.html, "&alpha;");
    assert_eq!(e.utf8, "α");
    assert!(e.use_brackets);
    let Event::Entity(e) = &ev[2].0 else {
        panic!("{ev:?}")
    };
    assert_eq!(e.name, "nbsp");
    assert!(!e.use_brackets);
    assert_eq!(e.utf8, "\u{a0}");
}

#[test]
fn leaf_fields() {
    let ev = events(
        "call_f[:a](x)[:b] src_py[:x 1]{y} {{{Key(a,b)}}} @@html:<b>@@ [fn:lbl] <<t>> [1/3]",
    );
    let leaves: Vec<_> = ev
        .into_iter()
        .filter(|(e, _)| !matches!(e, Event::Text(_)))
        .map(|(e, _)| e)
        .collect();
    assert_eq!(
        leaves,
        vec![
            Event::InlineBabelCall {
                call: "f".into(),
                inside_header: Some(":a".into()),
                arguments: Some("x".into()),
                end_header: Some(":b".into()),
            },
            Event::InlineSrcBlock {
                language: "py".into(),
                parameters: Some(":x 1".into()),
                value: "y".into(),
            },
            Event::Macro {
                key: "key".into(),
                args: vec!["a".into(), "b".into()],
            },
            Event::ExportSnippet {
                backend: "html".into(),
                value: "<b>".into(),
            },
            Event::FootnoteReference("lbl".into()),
            Event::Target("t".into()),
            Event::StatisticsCookie("[1/3]".into()),
        ]
    );
}

#[test]
fn citation_fields() {
    let ev = events("[cite/s:pre;@a s1; pp @b s2;suf]");
    assert_eq!(
        ev,
        vec![
            (
                Event::Start(Tag::Citation {
                    style: Some("s".into()),
                    prefix: Some("pre".into()),
                    suffix: Some("suf".into()),
                }),
                0..32
            ),
            (
                Event::CitationReference {
                    key: "a".into(),
                    prefix: None,
                    suffix: Some(" s1".into()),
                },
                12..18
            ),
            (
                Event::CitationReference {
                    key: "b".into(),
                    prefix: Some(" pp ".into()),
                    suffix: Some(" s2".into()),
                },
                18..28
            ),
            (Event::End(TagEnd::Citation), 0..32),
        ]
    );
}

#[test]
fn footnote_inline_definition() {
    let ev = events("[fn:x:a *b*]");
    assert_eq!(
        ev[0],
        (
            Event::Start(Tag::FootnoteReference {
                label: Some("x".into())
            }),
            0..12
        )
    );
    assert_eq!(ev[1], (Event::Text("a ".into()), 6..8));
    assert_eq!(
        ev.last().unwrap(),
        &(Event::End(TagEnd::FootnoteReference), 0..12)
    );
}

#[test]
fn sub_superscripts_option() {
    let src = "a_b c^{d} e_{f}";
    let mut settings = Settings::default();
    assert_eq!(
        render_with(src, &settings, Restriction::Standard),
        r#""a" (subscript 1 3 nil "b") " c" (superscript 5 9 t "d") " e" (subscript 11 15 t "f")"#
    );
    settings.sub_superscripts = SubSuperscripts::Braces;
    assert_eq!(
        render_with(src, &settings, Restriction::Standard),
        r#""a_b c" (superscript 5 9 t "d") " e" (subscript 11 15 t "f")"#
    );
    settings.sub_superscripts = SubSuperscripts::Never;
    assert_eq!(
        render_with(src, &settings, Restriction::Standard),
        r#""a_b c^{d} e_{f}""#
    );
    // Underline still wins over subscript.
    assert_eq!(
        render_with("x _u_", &settings, Restriction::Standard),
        r#""x " (underline 2 5 "u")"#
    );
}

#[test]
fn restrictions() {
    let settings = Settings::default();
    let src = "*b* [[l]] a\\\\\n[fn:1] <2024-01-01> [1/2] <<t>> call_f() {{{m}}}";
    let r = |r| render_with(src, &settings, r);
    // Link descriptions: no links, footnotes, timestamps, targets, line breaks.
    assert_eq!(
        r(Restriction::LinkDescription),
        r#"(bold 0 3 "b") " [[l]] a\\\\\n[fn:1] <2024-01-01> " (statistics-cookie 34 39 "[1/2]") " <<t>> " (inline-babel-call 46 54 f - - -) " " (macro 55 62 m)"#
    );
    // Radio targets: minimal set.
    assert_eq!(
        r(Restriction::RadioTarget),
        r#"(bold 0 3 "b") " [[l]] a\\\\\n[fn:1] <2024-01-01> [1/2] <<t>> call" (subscript 50 52 nil "f") "() {{{m}}}""#
    );
    // Titles and item tags: no line break.
    assert!(!r(Restriction::HeadlineTitle).contains("line-break"));
    assert!(!r(Restriction::ItemTag).contains("line-break"));
    assert!(r(Restriction::Standard).contains("line-break"));
    // Keywords: no footnote references.
    assert!(!r(Restriction::Keyword).contains("footnote-reference"));
    assert!(r(Restriction::Markup).contains("footnote-reference"));
    // Table cells: no cookie, babel call, line break.
    let t = r(Restriction::TableCell);
    assert!(
        !t.contains("statistics-cookie")
            && !t.contains("inline-babel-call")
            && !t.contains("line-break")
    );
    assert!(t.contains("(link") && t.contains("(target") && t.contains("(timestamp"));
}

#[test]
fn radio_one_char_at_line_start_terminates() {
    // Emacs loops forever on this input; we must not.
    let src = "<<<a>>>\na[[x]]";
    let out = render(src);
    assert!(out.starts_with(r#"(radio-target 0 7 "a")"#), "{out}");
}

#[test]
fn deep_nesting_does_not_overflow() {
    let n = 3000;
    let src = format!("{}a{}", "*".repeat(n), "*".repeat(n));
    let ev = events(&src);
    let starts = ev
        .iter()
        .filter(|(e, _)| matches!(e, Event::Start(Tag::Bold)))
        .count();
    assert!(starts > 1000);
    let src = format!("{}a{}", "[fn::".repeat(n), "]".repeat(n));
    let ev = events(&src);
    assert!(ev.len() > n);
    let src = "[".repeat(n) + &"]".repeat(n);
    let _ = events(&src);
}

/// A tiny deterministic PRNG (xorshift).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn soup_never_panics() {
    const PIECES: &[&str] = &[
        "a",
        "b",
        " ",
        "\n",
        "\t",
        "*",
        "/",
        "_",
        "+",
        "~",
        "=",
        "^",
        "$",
        "\\",
        "[",
        "]",
        "{",
        "}",
        "(",
        ")",
        "<",
        ">",
        "@",
        "%",
        ":",
        ";",
        ".",
        ",",
        "-",
        "'",
        "\"",
        "!",
        "?",
        "#",
        "|",
        "0",
        "1",
        "9",
        "é",
        "中",
        "α",
        "😀",
        "\u{a0}",
        "http://",
        "file:",
        "fn:",
        "cite:",
        "cite/s:",
        "@k",
        "@@",
        "<<",
        ">>",
        "<<<",
        ">>>",
        "{{{",
        "}}}",
        "src_",
        "call_",
        "\\alpha",
        "<2024-01-01",
        "[2024-01-01 Mon 10:00",
        "+1w",
        "--",
        "\\\\",
        "[[",
        "]]",
        "][",
        "\\(",
        "\\)",
        "$$",
        "_{",
        "^{",
        "<%%(",
        "a b",
    ];
    let restrictions = [
        Restriction::Standard,
        Restriction::HeadlineTitle,
        Restriction::LinkDescription,
        Restriction::TableCell,
        Restriction::ItemTag,
        Restriction::Keyword,
        Restriction::Markup,
        Restriction::SubSuperscript,
        Restriction::RadioTarget,
        Restriction::FootnoteReference,
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..3000 {
        let n = (rng.next() % 50) as usize;
        let mut s = String::new();
        for _ in 0..n {
            s.push_str(PIECES[(rng.next() % PIECES.len() as u64) as usize]);
        }
        let mut settings = snippet_settings(&s);
        if i % 3 == 0 {
            settings.radio_targets.push("a b".into());
            settings.radio_targets.push("é".into());
            settings.radio_targets.push("a".into());
        }
        settings.sub_superscripts = match i % 3 {
            0 => SubSuperscripts::Always,
            1 => SubSuperscripts::Braces,
            _ => SubSuperscripts::Never,
        };
        let r = restrictions[i % restrictions.len()];
        let mut out = Vec::new();
        parse_objects(&s, 0..s.len(), r, &settings, &mut out);
        render_events(&s, 0..s.len(), &out);
        // Also on a sub-range starting and ending on char boundaries.
        let mut a = s.len() / 3;
        while !s.is_char_boundary(a) {
            a += 1;
        }
        let mut out = Vec::new();
        parse_objects(&s, a..s.len(), r, &settings, &mut out);
        render_events(&s, a..s.len(), &out);
    }
}

#[test]
fn adversarial_inputs() {
    for src in [
        "\\",
        "$",
        "$$",
        "$$$$$$$$$$$$",
        "$a$$b$$c$",
        "\\\\\\\\\\",
        "[[[[[[[[",
        "]]]]]]",
        "<<<<<<>>>>>>",
        "{{{{{{}}}}}}",
        "@@@@@@",
        "*é* *中* é*a* 中_a 中^a a_中 \\é",
        "[fn:é:中]",
        "[cite:@é;@中]",
        "<2024-01-01 中>",
        "src_中{é} call_é(中)",
        "[[中][é]]",
        "_{_{_{_{_{",
        "^^^^^",
        "a_{{{{",
        "x^(((((",
        "😀*😀*😀",
    ] {
        let _ = render(src);
    }
}

#[test]
fn large_input_is_linear_enough() {
    let unit =
        "Some *bold* text, a [[https://example.com][link]], x_{i} and \\alpha <2024-01-01 Mon>.\n";
    let src = unit.repeat(5000);
    let start = std::time::Instant::now();
    let ev = events(&src);
    assert!(ev.len() > 50_000);
    assert!(start.elapsed().as_secs() < 20);
    let mut settings = snippet_settings(&src);
    settings.radio_targets.push("text".into());
    let mut out = Vec::new();
    parse_objects(
        &src,
        0..src.len(),
        Restriction::Standard,
        &settings,
        &mut out,
    );
    assert!(out.len() > ev.len());
}

#[test]
fn adversarial_inputs_are_not_quadratic() {
    // Many openings without closings: searches are cached per container.
    for unit in ["*a ", "/a ", "=a ", "\\( ", "\\[ ", "$$a ", "@@b:a ", "{{{a(", "<<<a ", "a_{", "[[a "] {
        let src = unit.repeat(20_000);
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        parse_objects(&src, 0..src.len(), Restriction::Standard, &Settings::default(), &mut out);
        assert!(start.elapsed().as_secs() < 5, "too slow on {unit:?}");
    }
}
