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
