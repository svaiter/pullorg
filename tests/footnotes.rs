//! HTML footnote section: only definitions reachable from a rendered
//! reference are exported (issue #8).

use pullorg::{html, Parser};

fn render(text: &str) -> String {
    let mut out = String::new();
    html::push_html(&mut out, Parser::for_export(text));
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
fn nested_references_are_numbered_depth_first() {
    let out =
        render("A [fn:a] B [fn:b].\n\n[fn:a] Def a [fn:c].\n\n[fn:b] Def b.\n\n[fn:c] Def c.\n");
    let defs = footdefs(&out);
    assert_eq!(defs.len(), 3);
    assert!(defs[0].starts_with("Def a "));
    assert!(defs[0].contains("id=\"fnr.2\" class=\"footref\" href=\"#fn.2\""));
    assert_eq!(defs[1], "Def c.");
    assert_eq!(defs[2], "Def b.");
    assert!(out.contains("B <sup><a id=\"fnr.3\""));
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
    assert!(defs[0].contains("id=\"fnr.1.101\" class=\"footref\" href=\"#fn.1\""));
    assert!(defs[1].contains("id=\"fnr.1.100\" class=\"footref\" href=\"#fn.1\""));
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
fn last_visible_definition_of_a_label_wins() {
    let out = render(
        "Text [fn:a].\n\n[fn:a] First [fn:unused].\n\n[fn:a] Second.\n\n[fn:unused] Lost.\n",
    );
    assert_eq!(footdefs(&out), ["Second."]);
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

#[test]
fn named_inline_definitions_can_be_reached_through_other_notes() {
    let out = render("See [fn:x].\n\n[fn:unused] Unused [fn:x:Actual X].\n");
    assert_eq!(footdefs(&out), ["Actual X"]);
    assert!(!out.contains("Unused"));

    let out = render("See [fn:x] and [fn:y].\n\n[fn:y] Contains [fn:x:Actual X].\n");
    let defs = footdefs(&out);
    assert_eq!(defs[0], "Actual X");
    assert!(defs[1].contains("href=\"#fn.1\""));
}

#[test]
fn inline_and_standalone_definitions_share_source_order_precedence() {
    assert_eq!(
        footdefs(&render("Body [fn:x:First] and [fn:x:Second].\n")),
        ["Second"]
    );
    assert_eq!(
        footdefs(&render("Body [fn:x:Inline].\n\n[fn:x] Standalone.\n")),
        ["Standalone."]
    );
    assert_eq!(
        footdefs(&render("[fn:x] Standalone.\n\n\nBody [fn:x:Inline].\n")),
        ["Inline"]
    );
}

#[test]
fn macros_and_counters_observe_source_order() {
    let out = render("[fn:a] Definition.\n\n#+MACRO: nick Ada\n\n\nHello {{{nick}}} [fn:a].\n");
    assert!(out.contains("Hello Ada "));

    let out =
        render("[fn:a] Note counter {{{n(seq)}}}.\n\n\nBody counter {{{n(seq)}}} with [fn:a].\n");
    assert_eq!(footdefs(&out), ["Note counter 1."]);
    assert!(out.contains("Body counter 2"));

    let out = render(
        "#+MACRO: name early\n\n[fn:a] {{{name}}}.\n\n\n#+MACRO: name late\n\nBody [fn:a].\n",
    );
    assert_eq!(footdefs(&out), ["early."]);
}

#[test]
fn unused_definitions_do_not_claim_visible_radio_anchors() {
    let out = render("[fn:unused] <<<word>>>.\n\n\nSee word and <<<word>>>.\n");
    assert!(out.contains("<a href=\"#org-radio-1\">word</a>"), "{out}");
    assert!(out.contains("<a id=\"org-radio-1\">word</a>"), "{out}");
    assert!(!out.contains("footdef"));
}

#[test]
fn lookahead_in_tables_accounts_for_radio_and_footnote_references() {
    let out = render("| radio | [fn:a] | [fn:b] |\n\n<<<radio>>>.\n\n[fn:a] A sees [fn:c].\n\n[fn:b] B.\n\n[fn:c] C.\n");
    assert!(
        out.contains("<td><a href=\"#org-radio-1\">radio</a></td>"),
        "{out}"
    );
    assert!(out.contains("<a id=\"org-radio-1\">radio</a>"), "{out}");
    assert!(out.contains("<td><sup><a id=\"fnr.3\""), "{out}");
    let defs = footdefs(&out);
    assert_eq!(&defs[1..], ["C.", "B."]);
}

#[test]
fn hidden_subtrees_do_not_override_visible_definitions_or_shift_references() {
    let out = render("See [fn:a].\n\n[fn:a] Visible.\n\n* Hidden :noexport:\n[fn:a] Hidden [fn:lost].\n\n* Shown\nAnother [fn:b:Kept].\n");
    assert_eq!(footdefs(&out), ["Visible.", "Kept"]);
    assert!(!out.contains("Hidden"));
}

#[test]
fn long_chain_of_definitions_is_resolved_without_recursion() {
    use std::fmt::Write;
    let mut text = String::from("See [fn:0].\n");
    for n in 0..4095 {
        writeln!(text, "\n[fn:{n}] See [fn:{}].", n + 1).unwrap();
    }
    text.push_str("\n[fn:4095] Last.\n");
    let out = render(&text);
    let defs = footdefs(&out);
    assert_eq!(defs.len(), 4096);
    assert_eq!(defs.last(), Some(&"Last."));
}
