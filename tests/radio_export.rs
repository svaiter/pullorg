//! Org's document parser and HTML exporter discover radio targets at different times.

use pullorg::{html, Event, Parser, Tag};

fn radio_paths(parser: Parser<'_>) -> Vec<String> {
    parser
        .filter_map(|event| match event {
            Event::Start(Tag::Link(link)) if link.link_type == "radio" => {
                Some(link.path.into_owned())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn comment_target_is_a_document_link_but_has_no_export_destination() {
    let src = "* COMMENT hidden\n<<<term>>>\n* Body\nterm.\n";
    assert_eq!(radio_paths(Parser::new(src)), ["term"]);
    assert_eq!(
        radio_paths(Parser::new_ext(src, Default::default())),
        ["term"]
    );
    assert!(radio_paths(Parser::for_export(src)).is_empty());
    let mut out = String::new();
    html::push_html(&mut out, Parser::for_export(src));
    assert!(out.contains("<p>term.</p>"), "{out}");
    assert!(!out.contains("org-radio-"), "{out}");
}

#[test]
fn export_refreshes_a_target_exposed_by_a_radio_link() {
    let src = "<<<a =b>>>\n\na =b <<<c>>>= x\n\nlater c.\n";
    assert_eq!(radio_paths(Parser::new(src)), ["a =b"]);
    assert_eq!(radio_paths(Parser::for_export(src)), ["a =b", "c"]);
    let mut out = String::new();
    html::push_html(&mut out, Parser::for_export(src));
    assert!(out.contains("<a href=\"#org-radio-2\">c</a>"), "{out}");
}
