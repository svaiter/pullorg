//! HTML footnote section: only definitions reachable from a rendered
//! reference are exported (issue #8).

use pullorg::{html, Parser};

fn render(text: &str) -> String {
    let mut out = String::new();
    html::push_html(&mut out, Parser::new(text));
    out
}

fn footdefs(out: &str) -> Vec<&str> {
    out.lines()
        .filter_map(|l| l.split_once("<div class=\"footpara\">"))
        .map(|(_, rest)| rest.trim_end_matches("</div></div>"))
        .collect()
}

#[test]
fn unused_definition_does_not_export_its_references() {
    let out = render(
        "Hello [fn:real].\n\n[fn:unused] Unused [fn:extra].\n\n[fn:real] Actual.\n\n[fn:extra] Extra.\n",
    );
    assert_eq!(footdefs(&out), ["Actual."]);
    assert!(!out.contains("Extra"));
    assert!(!out.contains("Unused"));
    assert!(out.contains("id=\"fnr.1\""));
    assert!(!out.contains("fnr.2"));
    assert!(!out.contains("fn.2"));
}

#[test]
fn unused_definitions_are_not_exported() {
    assert_eq!(render("[fn:x] Lost.\n"), "");
    assert_eq!(
        render("Text.\n\n[fn:x] Lost.\n\n[fn:y] Also lost.\n"),
        "<p>Text.</p>\n"
    );
}

#[test]
fn references_inside_definitions_are_numbered_after_the_body() {
    let out =
        render("A [fn:a] B [fn:b].\n\n[fn:a] Def a [fn:c].\n\n[fn:b] Def b.\n\n[fn:c] Def c.\n");
    let defs = footdefs(&out);
    assert_eq!(defs.len(), 3);
    assert!(defs[0].starts_with("Def a "));
    assert!(defs[0].contains("id=\"fnr.3\" class=\"footref\" href=\"#fn.3\""));
    assert_eq!(defs[1], "Def b.");
    assert_eq!(defs[2], "Def c.");
    assert!(out.contains("<a id=\"fn.3\" class=\"footnum\" href=\"#fnr.3\">3</a>"));
}

#[test]
fn every_backlink_target_exists() {
    let out = render(
        "A [fn:a] B [fn:b] A [fn:a].\n\n[fn:a] Def a [fn:c] [fn:b].\n\n[fn:b] Def b [fn::inline [fn:d]].\n\n[fn:c] Def c.\n\n[fn:d] Def d.\n\n[fn:e] Unused [fn:f].\n\n[fn:f] Also unused.\n",
    );
    let ids: Vec<&str> = out
        .split("id=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    let hrefs: Vec<&str> = out
        .split("href=\"#")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    assert!(!hrefs.is_empty());
    for h in &hrefs {
        assert!(ids.contains(h), "dangling backlink #{h} in:\n{out}");
    }
    assert_eq!(footdefs(&out).len(), 5);
    assert!(!out.contains("unused"));
}

#[test]
fn cyclic_definitions_render_once() {
    let out = render("X [fn:a].\n\n[fn:a] A sees [fn:b] and [fn:a].\n\n[fn:b] B sees [fn:a].\n");
    let defs = footdefs(&out);
    assert_eq!(defs.len(), 2);
    assert!(defs[0].starts_with("A sees "));
    assert!(defs[1].starts_with("B sees "));
    // Back-references reuse the existing number with a distinct id.
    assert!(defs[0].contains("id=\"fnr.1.100\" class=\"footref\" href=\"#fn.1\""));
    assert!(defs[1].contains("id=\"fnr.1.101\" class=\"footref\" href=\"#fn.1\""));
    assert!(out.contains("id=\"fn.1\"") && out.contains("id=\"fn.2\""));
    assert!(!out.contains("fn.3"));
}

#[test]
fn definition_before_its_reference_is_found() {
    // Two blank lines end the definition.
    let out = render("[fn:a] Early.\n\n\nText [fn:a].\n");
    assert_eq!(footdefs(&out), ["Early."]);
}

#[test]
fn first_definition_of_a_label_wins() {
    let out = render("Text [fn:a].\n\n[fn:a] First.\n\n[fn:a] Second.\n");
    assert_eq!(footdefs(&out), ["First."]);
}

#[test]
fn multi_paragraph_definition_keeps_its_structure() {
    let out = render("Text [fn:a].\n\n[fn:a] One.\n\nTwo.\n\n| a | b |\n");
    assert_eq!(out.matches("class=\"footdef\"").count(), 1);
    assert!(
        out.contains("<div class=\"footpara\">One.\n<p>Two.</p>\n<table>"),
        "{out}"
    );
}

#[test]
fn definitions_in_footnote_section_still_resolve() {
    let out = render("Text [fn:a].\n\n* Footnotes\n\n[fn:a] From section.\n\n[fn:b] Unused.\n");
    assert_eq!(footdefs(&out), ["From section."]);
    assert!(!out.contains("<h2 id"));
}

#[test]
fn definitions_under_noexport_are_unavailable() {
    let out = render("Text [fn:a].\n\n* Hidden :noexport:\n\n[fn:a] Gone.\n");
    assert_eq!(footdefs(&out), [""]);
}
