//! Regression inputs for repeated closing-delimiter searches (issue #5).

use std::fmt::Write;

use pullorg::{Event, Options, Parser, SubSuperscripts, Tag};

#[test]
fn many_unterminated_blocks_and_drawers_remain_paragraph_text() {
    for distinct in [false, true] {
        for drawers in [false, true] {
            let mut text = String::new();
            for i in 0..20_000 {
                let name = if distinct { i } else { 0 };
                if drawers {
                    writeln!(text, ":drawer{name}:").unwrap();
                } else {
                    writeln!(text, "#+begin_block{name}").unwrap();
                }
            }
            let mut plain = String::new();
            let mut paragraphs = 0;
            let mut options = Options::default();
            options.sub_superscripts = SubSuperscripts::Never;
            for event in Parser::new_ext(&text, options) {
                match event {
                    Event::Start(Tag::Paragraph) => paragraphs += 1,
                    Event::Text(s) => plain.push_str(&s),
                    Event::SoftBreak => plain.push('\n'),
                    Event::Start(Tag::SpecialBlock { .. } | Tag::Drawer { .. }) => {
                        panic!("unterminated opener became a container");
                    }
                    _ => {}
                }
            }
            assert_eq!(paragraphs, 1);
            assert!(
                plain == text.trim_end_matches('\n'),
                "paragraph text changed"
            );
        }
    }
}

#[test]
fn unterminated_delimiters_inside_a_list_do_not_rescan_the_suffix() {
    for opener in [
        "#+begin_x",
        ":DRAWER:",
        "#+BEGIN: clocktable",
        "\\begin{equation}",
    ] {
        let text = format!(
            "- item\n{}- after\n",
            format!("  {opener}\n").repeat(10_000)
        );
        let mut items = 0;
        for event in Parser::new(&text) {
            match event {
                Event::Start(Tag::Item(_)) => items += 1,
                Event::Start(
                    Tag::SpecialBlock { .. } | Tag::Drawer { .. } | Tag::DynamicBlock { .. },
                )
                | Event::LatexEnvironment(_) => panic!("unterminated opener became a container"),
                _ => {}
            }
        }
        assert_eq!(items, 2, "{opener}");
    }
}

#[test]
fn long_backslash_runs_preserve_link_paths_and_ranges() {
    for n in [40_000, 40_001] {
        for bracket in [false, true] {
            let text = format!(
                "[[{}{}",
                "\\".repeat(n),
                if bracket { "]]]" } else { "a]]" }
            );
            let expected = if bracket {
                format!(
                    "{}{}",
                    "\\".repeat(n / 2),
                    if n % 2 == 1 { "]" } else { "" }
                )
            } else {
                format!("{}a", "\\".repeat(n))
            };
            let links: Vec<_> = Parser::new_inline(&text)
                .into_offset_iter()
                .filter_map(|(event, range)| match event {
                    Event::Start(Tag::Link(link)) => Some((link.path, range)),
                    _ => None,
                })
                .collect();
            assert_eq!(links.len(), 1);
            assert!(links[0].0 == expected, "link unescaping changed");
            assert_eq!(
                links[0].1,
                0..text.len() - usize::from(bracket && n % 2 == 0)
            );
        }
    }
}

#[test]
fn failed_link_openers_do_not_hide_the_final_valid_link() {
    for unit in ["[[a", "[[a\\["] {
        let text = format!("{} [[valid][description]]", unit.repeat(30_000));
        let links: Vec<_> = Parser::new_inline(&text)
            .filter_map(|event| match event {
                Event::Start(Tag::Link(link)) => Some(link.path),
                _ => None,
            })
            .collect();
        assert_eq!(links, ["valid"]);
    }
}
