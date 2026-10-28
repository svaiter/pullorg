//! Conformance tests against Emacs' `org-element.el`.
//!
//! Every `tests/fixtures/NAME.org` has a `NAME.json` produced by Emacs (see
//! `tests/emacs/dump.el` and `tools/regen-fixtures.sh`). This test parses the
//! `.org` file with pullorg, converts the event stream into the same
//! JSON shape, normalizes both trees and compares them.
//!
//! Set `PULLORG_FIXTURE=name[,name...]` to run only some fixtures.
//! See `tests/README.md` for the JSON shape and the normalization rules.

#![recursion_limit = "256"]

use std::fs;
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use pullorg::{
    Checkbox, ClockStatus, Event, Headline, Item, Keyword, Link, LinkFormat, ListKind, Parser,
    Priority, RepeaterKind, TableKind, TableRowKind, Tag, TagEnd, TimeUnit, Timestamp,
    TimestampKind, TodoType, WarningKind,
};
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Event stream -> JSON tree
// ---------------------------------------------------------------------------

/// What a frame of the conversion stack collects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameKind {
    /// The `org-data` root.
    Root,
    /// A regular node: children go to `children`.
    Node,
    /// A node whose single `Text` content is its `value` property
    /// (src/example/export/comment blocks, table.el tables).
    Raw,
    /// `Start(Title)`: children go to the parent's `props.title`.
    Title,
    /// `Start(ItemTag)`: children go to the parent's `props.tag`.
    ItemTag,
}

struct Frame {
    kind: FrameKind,
    end: Option<TagEnd>,
    node: Map<String, Value>,
    children: Vec<Value>,
}

impl Frame {
    fn new(kind: FrameKind, end: Option<TagEnd>, node: Map<String, Value>) -> Self {
        Frame {
            kind,
            end,
            node,
            children: Vec::new(),
        }
    }
}

fn s(x: &str) -> Value {
    Value::String(x.to_string())
}

fn opt(x: &Option<impl AsRef<str>>) -> Value {
    match x {
        Some(v) => s(v.as_ref()),
        None => Value::Null,
    }
}

fn node(ty: &str, range: &Range<usize>, props: Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("type".into(), s(ty));
    m.insert("begin".into(), json!(range.start));
    m.insert("end".into(), json!(range.end));
    m.insert("props".into(), props);
    m
}

fn plain_text(text: &str) -> Value {
    json!({"type": "plain-text", "value": text})
}

/// Append `child` to `children`, merging adjacent plain-text nodes.
fn push_child(children: &mut Vec<Value>, child: Value) {
    if child["type"] == "plain-text" {
        if let Some(last) = children.last_mut() {
            if last["type"] == "plain-text" {
                let merged = format!(
                    "{}{}",
                    last["value"].as_str().unwrap_or(""),
                    child["value"].as_str().unwrap_or("")
                );
                last["value"] = Value::String(merged);
                return;
            }
        }
    }
    children.push(child);
}

fn todo_type(t: &Option<TodoType>) -> Value {
    match t {
        Some(TodoType::Todo) => s("todo"),
        Some(TodoType::Done) => s("done"),
        None => Value::Null,
    }
}

fn headline_props(h: &Headline<'_>) -> Value {
    json!({
        "level": h.level,
        "todo-keyword": opt(&h.todo_keyword),
        "todo-type": todo_type(&h.todo_type),
        "priority": h.priority.map(|p| p.value()),
        "tags": h.tags.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
        "commentedp": h.commented,
        "archivedp": h.archived,
        "footnote-section-p": h.footnote_section,
        "raw-value": h.raw_title.as_ref(),
        "title": [],
    })
}

/// Emacs' `:counter`: a number, or the rank of a letter (`c` -> 3).
fn item_counter(counter: &Option<impl AsRef<str>>) -> Value {
    let Some(c) = counter else {
        return Value::Null;
    };
    let c = c.as_ref();
    if let Some(l) = c.chars().find(|c| c.is_ascii_alphabetic()) {
        return json!(l.to_ascii_uppercase() as u32 - 64);
    }
    let digits: String = c
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits
        .parse::<u64>()
        .map(|n| json!(n))
        .unwrap_or(Value::Null)
}

fn item_props(i: &Item<'_>) -> Value {
    json!({
        "bullet": i.bullet.as_ref(),
        "checkbox": match i.checkbox {
            Some(Checkbox::Off) => s("off"),
            Some(Checkbox::On) => s("on"),
            Some(Checkbox::Trans) => s("trans"),
            None => Value::Null,
        },
        "counter": item_counter(&i.counter),
        "tag": Value::Null,
    })
}

fn time_unit(u: TimeUnit) -> &'static str {
    match u {
        TimeUnit::Hour => "hour",
        TimeUnit::Day => "day",
        TimeUnit::Week => "week",
        TimeUnit::Month => "month",
        TimeUnit::Year => "year",
    }
}

fn timestamp_props(ts: &Timestamp<'_>) -> Value {
    let kind = match ts.kind {
        TimestampKind::Active => "active",
        TimestampKind::Inactive => "inactive",
        TimestampKind::ActiveRange => "active-range",
        TimestampKind::InactiveRange => "inactive-range",
        TimestampKind::Diary => "diary",
    };
    let mut m = Map::new();
    m.insert("type".into(), s(kind));
    m.insert("raw-value".into(), s(&ts.raw));
    // Emacs sets the *-end properties to the start values for non-ranges.
    let end = ts.end.or(ts.start);
    for (suffix, dt) in [("start", ts.start), ("end", end)] {
        m.insert(format!("year-{suffix}"), json!(dt.map(|d| d.year)));
        m.insert(format!("month-{suffix}"), json!(dt.map(|d| d.month)));
        m.insert(format!("day-{suffix}"), json!(dt.map(|d| d.day)));
        m.insert(format!("hour-{suffix}"), json!(dt.and_then(|d| d.hour)));
        m.insert(format!("minute-{suffix}"), json!(dt.and_then(|d| d.minute)));
    }
    let r = ts.repeater;
    m.insert(
        "repeater-type".into(),
        json!(r.map(|r| match r.kind {
            RepeaterKind::Cumulate => "cumulate",
            RepeaterKind::CatchUp => "catch-up",
            RepeaterKind::Restart => "restart",
        })),
    );
    m.insert("repeater-value".into(), json!(r.map(|r| r.value)));
    m.insert("repeater-unit".into(), json!(r.map(|r| time_unit(r.unit))));
    let w = ts.warning;
    m.insert(
        "warning-type".into(),
        json!(w.map(|w| match w.kind {
            WarningKind::All => "all",
            WarningKind::First => "first",
        })),
    );
    m.insert("warning-value".into(), json!(w.map(|w| w.value)));
    m.insert("warning-unit".into(), json!(w.map(|w| time_unit(w.unit))));
    Value::Object(m)
}

/// A timestamp stored in a property (planning, clock): no positions.
fn embedded_timestamp(ts: &Option<Timestamp<'_>>) -> Value {
    match ts {
        Some(ts) => json!({"type": "timestamp", "props": timestamp_props(ts), "children": []}),
        None => Value::Null,
    }
}

fn link_props(l: &Link<'_>) -> Value {
    json!({
        "type": l.link_type.as_ref(),
        "path": l.path.as_ref(),
        "format": match l.format {
            LinkFormat::Plain => "plain",
            LinkFormat::Angle => "angle",
            LinkFormat::Bracket => "bracket",
        },
        "raw-link": l.raw_link.as_ref(),
        "search-option": opt(&l.search_option),
        "application": opt(&l.application),
    })
}

/// `org-element-keyword-translation-alist`.
fn translate_keyword(key: &str) -> String {
    let key = key.to_ascii_uppercase();
    match key.as_str() {
        "DATA" | "LABEL" | "RESNAME" | "SOURCE" | "SRCNAME" | "TBLNAME" => "NAME".into(),
        "RESULT" => "RESULTS".into(),
        "HEADERS" => "HEADER".into(),
        _ => key,
    }
}

fn affiliated_entry(k: &Keyword<'_>) -> Value {
    json!({
        "key": translate_keyword(&k.key),
        "value": k.value.trim(),
        "optional": opt(&k.optional),
    })
}

fn sort_affiliated(entries: &mut [Value]) {
    let key = |v: &Value| {
        (
            v["key"].as_str().unwrap_or("").to_string(),
            v["value"].as_str().unwrap_or("").to_string(),
            v["optional"].as_str().unwrap_or("").to_string(),
        )
    };
    entries.sort_by_key(key);
}

/// Convert a start tag to `(frame kind, node)`.
fn start_node(tag: &Tag<'_>, range: &Range<usize>, src: &str) -> (FrameKind, Map<String, Value>) {
    use FrameKind::{Node, Raw};
    let n = |ty: &str, props: Value| node(ty, range, props);
    match tag {
        Tag::Headline(h) => (Node, n("headline", headline_props(h))),
        Tag::Inlinetask(h) => (Node, n("inlinetask", headline_props(h))),
        Tag::Title => (FrameKind::Title, Map::new()),
        Tag::ItemTag => (FrameKind::ItemTag, Map::new()),
        Tag::Section => (Node, n("section", json!({}))),
        Tag::Paragraph => (Node, n("paragraph", json!({}))),
        Tag::PlainList { kind } => (
            Node,
            n(
                "plain-list",
                json!({"type": match kind {
                    ListKind::Unordered => "unordered",
                    ListKind::Ordered => "ordered",
                    ListKind::Descriptive => "descriptive",
                }}),
            ),
        ),
        Tag::Item(i) => (Node, n("item", item_props(i))),
        Tag::Table { kind, formulas } => {
            let props = json!({
                "type": match kind { TableKind::Org => "org", TableKind::TableEl => "table.el" },
                // Emacs' `:tblfm` holds the lines last first; ours is in source order.
                "tblfm": formulas.iter().rev().map(|f| f.to_string()).collect::<Vec<_>>(),
                "value": Value::Null,
            });
            let fk = if *kind == TableKind::TableEl {
                Raw
            } else {
                Node
            };
            (fk, n("table", props))
        }
        Tag::TableRow { kind } => (
            Node,
            n(
                "table-row",
                json!({"type": match kind {
                    TableRowKind::Standard => "standard",
                    TableRowKind::Rule => "rule",
                }}),
            ),
        ),
        Tag::TableCell => (Node, n("table-cell", json!({}))),
        Tag::CenterBlock => (Node, n("center-block", json!({}))),
        Tag::QuoteBlock => (Node, n("quote-block", json!({}))),
        Tag::VerseBlock => (Node, n("verse-block", json!({}))),
        Tag::SpecialBlock { name, parameters } => (
            Node,
            n(
                "special-block",
                json!({"type": name.as_ref(), "parameters": opt(parameters)}),
            ),
        ),
        Tag::SrcBlock {
            language,
            switches,
            parameters,
        } => (
            Raw,
            n(
                "src-block",
                json!({
                    "language": opt(language),
                    "switches": opt(switches),
                    "parameters": opt(parameters),
                    "value": "",
                }),
            ),
        ),
        Tag::ExampleBlock { switches } => (
            Raw,
            n(
                "example-block",
                json!({"switches": opt(switches), "value": ""}),
            ),
        ),
        // Emacs upper-cases the backend of export blocks.
        Tag::ExportBlock { backend } => (
            Raw,
            n(
                "export-block",
                json!({"type": backend.to_uppercase(), "value": ""}),
            ),
        ),
        Tag::CommentBlock => (Raw, n("comment-block", json!({"value": ""}))),
        Tag::Drawer { name } => (Node, n("drawer", json!({"drawer-name": name.as_ref()}))),
        Tag::PropertyDrawer => (Node, n("property-drawer", json!({}))),
        Tag::DynamicBlock { name, parameters } => (
            Node,
            n(
                "dynamic-block",
                json!({"block-name": name.as_ref(), "arguments": opt(parameters)}),
            ),
        ),
        Tag::FootnoteDefinition { label } => (
            Node,
            n("footnote-definition", json!({"label": label.as_ref()})),
        ),
        Tag::Bold => (Node, n("bold", json!({}))),
        Tag::Italic => (Node, n("italic", json!({}))),
        Tag::Underline => (Node, n("underline", json!({}))),
        Tag::StrikeThrough => (Node, n("strike-through", json!({}))),
        Tag::Link(l) => (Node, n("link", link_props(l))),
        Tag::Subscript { use_brackets } => (
            Node,
            n("subscript", json!({"use-brackets-p": use_brackets})),
        ),
        Tag::Superscript { use_brackets } => (
            Node,
            n("superscript", json!({"use-brackets-p": use_brackets})),
        ),
        Tag::RadioTarget => {
            // Emacs' :value is the raw text between `<<<` and `>>>`.
            let raw = src.get(range.clone()).unwrap_or("");
            let value = raw
                .strip_prefix("<<<")
                .and_then(|r| r.strip_suffix(">>>"))
                .unwrap_or(raw);
            (Node, n("radio-target", json!({"value": value})))
        }
        Tag::FootnoteReference { label } => (
            Node,
            n(
                "footnote-reference",
                json!({"label": opt(label), "type": "inline"}),
            ),
        ),
        Tag::Citation {
            style,
            prefix,
            suffix,
        } => (
            Node,
            n(
                "citation",
                json!({"style": opt(style), "prefix": opt(prefix), "suffix": opt(suffix)}),
            ),
        ),
    }
}

fn babel_props(
    call: &str,
    inside: &Option<impl AsRef<str>>,
    args: &Option<impl AsRef<str>>,
    end: &Option<impl AsRef<str>>,
) -> Value {
    json!({
        "call": call,
        "inside-header": opt(inside),
        "arguments": opt(args),
        "end-header": opt(end),
    })
}

/// Convert a leaf event (anything but Start/End/Text/SoftBreak) to a node.
fn leaf_node(event: &Event<'_>, range: &Range<usize>) -> Option<Map<String, Value>> {
    let n = |ty: &str, props: Value| Some(node(ty, range, props));
    match event {
        Event::Code(v) => n("code", json!({"value": v.as_ref()})),
        Event::Verbatim(v) => n("verbatim", json!({"value": v.as_ref()})),
        Event::Entity(e) => n(
            "entity",
            json!({"name": e.name, "use-brackets-p": e.use_brackets}),
        ),
        Event::LatexFragment(v) => n("latex-fragment", json!({"value": v.as_ref()})),
        Event::ExportSnippet { backend, value } => n(
            "export-snippet",
            json!({"back-end": backend.as_ref(), "value": value.as_ref()}),
        ),
        Event::InlineSrcBlock {
            language,
            parameters,
            value,
        } => n(
            "inline-src-block",
            json!({"language": language.as_ref(), "parameters": opt(parameters), "value": value.as_ref()}),
        ),
        Event::InlineBabelCall {
            call,
            inside_header,
            arguments,
            end_header,
        } => n(
            "inline-babel-call",
            babel_props(call, inside_header, arguments, end_header),
        ),
        Event::Macro { key, args } => n(
            "macro",
            json!({"key": key.as_ref(), "args": args.iter().map(|a| a.to_string()).collect::<Vec<_>>()}),
        ),
        Event::Target(v) => n("target", json!({"value": v.as_ref()})),
        Event::StatisticsCookie(v) => n("statistics-cookie", json!({"value": v.as_ref()})),
        Event::Timestamp(ts) => n("timestamp", timestamp_props(ts)),
        Event::FootnoteReference(label) => n(
            "footnote-reference",
            json!({"label": label.as_ref(), "type": "standard"}),
        ),
        Event::CitationReference {
            key,
            prefix,
            suffix,
        } => n(
            "citation-reference",
            json!({"key": key.as_ref(), "prefix": opt(prefix), "suffix": opt(suffix)}),
        ),
        Event::LineBreak => n("line-break", json!({})),
        Event::Rule => n("horizontal-rule", json!({})),
        Event::Keyword(k) => n(
            "keyword",
            json!({"key": k.key.to_ascii_uppercase(), "value": k.value.as_ref()}),
        ),
        Event::Comment(v) => n("comment", json!({"value": v.as_ref()})),
        Event::FixedWidth(v) => n("fixed-width", json!({"value": v.as_ref()})),
        Event::LatexEnvironment(v) => n("latex-environment", json!({"value": v.as_ref()})),
        Event::BabelCall {
            call,
            inside_header,
            arguments,
            end_header,
        } => n(
            "babel-call",
            babel_props(call, inside_header, arguments, end_header),
        ),
        Event::DiarySexp(v) => n("diary-sexp", json!({"value": v.as_ref()})),
        Event::Clock {
            timestamp,
            duration,
            status,
        } => n(
            "clock",
            json!({
                "status": match status { ClockStatus::Closed => "closed", ClockStatus::Running => "running" },
                "duration": opt(duration),
                "value": embedded_timestamp(timestamp),
            }),
        ),
        Event::Planning {
            closed,
            scheduled,
            deadline,
        } => n(
            "planning",
            json!({
                "closed": embedded_timestamp(closed),
                "scheduled": embedded_timestamp(scheduled),
                "deadline": embedded_timestamp(deadline),
            }),
        ),
        Event::NodeProperty { key, value } => n(
            "node-property",
            json!({"key": key.as_ref(), "value": value.as_ref()}),
        ),
        Event::Start(_) | Event::End(_) | Event::Text(_) | Event::SoftBreak => None,
    }
}

/// Convert an event stream with byte ranges into the JSON tree shape
/// produced by `tests/emacs/dump.el`. `src` is the source text (used to
/// recover raw values the events do not carry, e.g. radio target text).
fn events_to_json<'a, I>(src: &str, events: I) -> Result<Value, String>
where
    I: IntoIterator<Item = (Event<'a>, Range<usize>)>,
{
    let mut root = Map::new();
    root.insert("type".into(), s("org-data"));
    let mut stack = vec![Frame::new(FrameKind::Root, None, root)];
    let mut pending_affiliated: Vec<Value> = Vec::new();

    fn take_affiliated(pending: &mut Vec<Value>, node: &mut Map<String, Value>) {
        if pending.is_empty() {
            return;
        }
        let mut entries = std::mem::take(pending);
        sort_affiliated(&mut entries);
        if let Some(Value::Object(props)) = node.get_mut("props") {
            props.insert("affiliated".into(), Value::Array(entries));
        }
    }

    for (index, (event, range)) in events.into_iter().enumerate() {
        let ctx = |msg: String| format!("event #{index} {event:?} @ {range:?}: {msg}");
        match &event {
            Event::Start(tag) => {
                let (kind, mut n) = start_node(tag, &range, src);
                if !matches!(kind, FrameKind::Title | FrameKind::ItemTag) {
                    take_affiliated(&mut pending_affiliated, &mut n);
                } else if !pending_affiliated.is_empty() {
                    return Err(ctx("affiliated keyword before a title/tag".into()));
                }
                stack.push(Frame::new(kind, Some(tag.to_end()), n));
            }
            Event::End(end) => {
                if !pending_affiliated.is_empty() {
                    return Err(ctx("dangling affiliated keyword(s)".into()));
                }
                let frame = stack.pop().ok_or_else(|| ctx("unbalanced End".into()))?;
                if frame.end != Some(*end) {
                    return Err(ctx(format!("mismatched End, open tag is {:?}", frame.end)));
                }
                let parent = stack
                    .last_mut()
                    .ok_or_else(|| ctx("End closes the root".into()))?;
                match frame.kind {
                    FrameKind::Title | FrameKind::ItemTag => {
                        let key = if frame.kind == FrameKind::Title {
                            "title"
                        } else {
                            "tag"
                        };
                        match parent.node.get_mut("props") {
                            Some(Value::Object(props)) => {
                                props.insert(key.into(), Value::Array(frame.children));
                            }
                            _ => return Err(ctx(format!("{key} outside of a node"))),
                        }
                    }
                    _ => {
                        let mut n = frame.node;
                        n.insert("children".into(), Value::Array(frame.children));
                        push_child(&mut parent.children, Value::Object(n));
                    }
                }
            }
            Event::Text(text) => {
                let top = stack.last_mut().unwrap();
                if top.kind == FrameKind::Raw {
                    if let Some(Value::Object(props)) = top.node.get_mut("props") {
                        let old = props
                            .get("value")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        props.insert("value".into(), Value::String(old + text));
                    }
                } else {
                    push_child(&mut top.children, plain_text(text));
                }
            }
            Event::SoftBreak => {
                push_child(&mut stack.last_mut().unwrap().children, plain_text("\n"));
            }
            Event::Keyword(k) if k.affiliated => {
                pending_affiliated.push(affiliated_entry(k));
            }
            leaf => {
                let mut n = leaf_node(leaf, &range).expect("leaf event");
                take_affiliated(&mut pending_affiliated, &mut n);
                n.insert("children".into(), Value::Array(Vec::new()));
                push_child(&mut stack.last_mut().unwrap().children, Value::Object(n));
            }
        }
    }
    if !pending_affiliated.is_empty() {
        return Err("dangling affiliated keyword(s) at end of document".into());
    }
    if stack.len() != 1 {
        let open: Vec<_> = stack.iter().skip(1).map(|f| f.end).collect();
        return Err(format!("unclosed containers at end of document: {open:?}"));
    }
    let root = stack.pop().unwrap();
    let mut n = root.node;
    n.insert("children".into(), Value::Array(root.children));
    Ok(Value::Object(n))
}

// ---------------------------------------------------------------------------
// Normalization (applied to both trees)
// ---------------------------------------------------------------------------

/// Types whose `value` property is a raw multi-line body: trailing newlines
/// are not compared (Emacs keeps the final newline, pullorg drops it).
const RAW_VALUE_TYPES: &[&str] = &[
    "src-block",
    "example-block",
    "export-block",
    "comment-block",
    "latex-environment",
    "table",
];

fn normalize_nodes(nodes: &mut Vec<Value>) {
    // Merge adjacent plain text.
    let old = std::mem::take(nodes);
    for mut n in old {
        normalize(&mut n);
        push_child(nodes, n);
    }
    // Trailing whitespace of the last plain-text child is not compared.
    if let Some(last) = nodes.last_mut() {
        if last["type"] == "plain-text" {
            let trimmed = last["value"].as_str().unwrap_or("").trim_end().to_string();
            if trimmed.is_empty() {
                nodes.pop();
            } else {
                last["value"] = Value::String(trimmed);
            }
        }
    }
}

fn normalize(v: &mut Value) {
    let Value::Object(n) = v else { return };
    let ty = n
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(Value::Array(children)) = n.get_mut("children") {
        normalize_nodes(children);
    }
    if let Some(Value::Object(props)) = n.get_mut("props") {
        // Absent and null are the same thing.
        props.retain(|_, v| !v.is_null());
        for key in ["title", "tag"] {
            if let Some(Value::Array(nodes)) = props.get_mut(key) {
                normalize_nodes(nodes);
            }
        }
        for key in ["closed", "scheduled", "deadline", "value"] {
            if let Some(ts @ Value::Object(_)) = props.get_mut(key) {
                normalize(ts);
            }
        }
        if RAW_VALUE_TYPES.contains(&ty.as_str()) {
            if let Some(Value::String(value)) = props.get_mut("value") {
                let t = value.trim_end_matches(['\n', '\r']).to_string();
                *value = t;
            }
        }
        if ty == "citation" || ty == "citation-reference" {
            for key in ["prefix", "suffix"] {
                if let Some(Value::String(x)) = props.get(key) {
                    let t = x.trim().to_string();
                    if t.is_empty() {
                        props.remove(key);
                    } else {
                        props.insert(key.into(), Value::String(t));
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture driver
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures_dir())
        .expect("tests/fixtures exists")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "org").then(|| p.file_stem()?.to_str().map(String::from))?
        })
        .collect();
    names.sort();
    if let Ok(filter) = std::env::var("PULLORG_FIXTURE") {
        let wanted: Vec<&str> = filter
            .split(',')
            .map(|n| n.trim().trim_end_matches(".org"))
            .filter(|n| !n.is_empty())
            .collect();
        for w in &wanted {
            assert!(
                names.iter().any(|n| n == w),
                "PULLORG_FIXTURE: no fixture named {w:?}"
            );
        }
        names.retain(|n| wanted.contains(&n.as_str()));
    }
    names
}

fn load_expected(name: &str) -> Result<Value, String> {
    let path = fixtures_dir().join(format!("{name}.json"));
    let text = fs::read_to_string(&path).map_err(|e| {
        format!(
            "cannot read {}: {e}\n  hint: run `sh tools/regen-fixtures.sh {name}` (needs Emacs)",
            path.display()
        )
    })?;
    serde_json::from_str(&text).map_err(|e| format!("invalid JSON in {}: {e}", path.display()))
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic>".into())
}

fn check_fixture(name: &str) -> Result<(), String> {
    let mut expected = load_expected(name)?;
    let org_path = fixtures_dir().join(format!("{name}.org"));
    let src = fs::read_to_string(&org_path).map_err(|e| format!("{}: {e}", org_path.display()))?;
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let events: Vec<_> = Parser::new(&src).into_offset_iter().collect();
        events_to_json(&src, events)
    }));
    let mut actual = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(format!("invalid event stream: {e}")),
        Err(p) => return Err(format!("parser panicked: {}", panic_message(&*p))),
    };
    normalize(&mut expected);
    normalize(&mut actual);
    if expected == actual {
        return Ok(());
    }
    let e = serde_json::to_string_pretty(&expected).unwrap();
    let a = serde_json::to_string_pretty(&actual).unwrap();
    Err(format!(
        "mismatch (left: Emacs, right: pullorg)\n{}",
        pretty_assertions::StrComparison::new(&e, &a)
    ))
}

#[test]
fn emacs_conformance() {
    let names = fixture_names();
    assert!(!names.is_empty(), "no fixtures found");
    // Silence the default panic output of the parser; panics are reported
    // per fixture below.
    let hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let results: Vec<(String, Result<(), String>)> = names
        .iter()
        .map(|n| (n.clone(), check_fixture(n)))
        .collect();
    panic::set_hook(hook);

    let failures: Vec<_> = results.iter().filter(|(_, r)| r.is_err()).collect();
    for (name, r) in &failures {
        eprintln!("\n===== fixture {name} =====\n{}", r.as_ref().unwrap_err());
    }
    if !failures.is_empty() {
        let list: Vec<_> = failures.iter().map(|(n, _)| n.as_str()).collect();
        panic!(
            "{} of {} fixture(s) failed: {}\n(rerun one with PULLORG_FIXTURE=name)",
            failures.len(),
            results.len(),
            list.join(", ")
        );
    }
}

/// Every fixture has a parseable, well-formed JSON oracle. Does not need the
/// parser.
#[test]
fn fixture_json_is_well_formed() {
    fn check(v: &Value, path: &str) -> Result<(), String> {
        let ty = v["type"]
            .as_str()
            .ok_or_else(|| format!("{path}: node without type"))?;
        if ty == "plain-text" {
            return v["value"]
                .as_str()
                .map(|_| ())
                .ok_or_else(|| format!("{path}: plain-text without value"));
        }
        let children = v["children"]
            .as_array()
            .ok_or_else(|| format!("{path}: {ty} without children"))?;
        if ty != "org-data" && ty != "timestamp" {
            let (b, e) = (v["begin"].as_u64(), v["end"].as_u64());
            match (b, e) {
                (Some(b), Some(e)) if b <= e => {}
                _ => return Err(format!("{path}: {ty} has bad positions")),
            }
        }
        for (i, c) in children.iter().enumerate() {
            check(c, &format!("{path}/{ty}[{i}]"))?;
        }
        Ok(())
    }
    let mut errors = Vec::new();
    for name in fixture_names() {
        match load_expected(&name) {
            Ok(v) => {
                if let Err(e) = check(&v, &name) {
                    errors.push(e)
                }
            }
            Err(e) => errors.push(e),
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

// ---------------------------------------------------------------------------
// Converter unit tests (hand-built event streams)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod converter {
    use super::*;
    use pretty_assertions::assert_eq;
    use pullorg::{DateTime, Repeater};
    use std::borrow::Cow;

    fn b(x: &str) -> Cow<'_, str> {
        Cow::Borrowed(x)
    }

    #[test]
    fn headline_paragraph_and_affiliated() {
        // * TODO [#A] Hi *b* :t:
        // #+NAME: p
        // Some
        // text <2024-01-01 Mon +1w>
        let src = "* TODO [#A] Hi *b* :t:\n#+NAME: p\nSome\ntext <2024-01-01 Mon +1w>\n";
        let ts = Timestamp {
            kind: TimestampKind::Active,
            raw: b("<2024-01-01 Mon +1w>"),
            start: Some(DateTime {
                year: 2024,
                month: 1,
                day: 1,
                hour: None,
                minute: None,
            }),
            end: None,
            repeater: Some(Repeater {
                kind: RepeaterKind::Cumulate,
                value: 1,
                unit: TimeUnit::Week,
            }),
            warning: None,
        };
        let events = vec![
            (
                Event::Start(Tag::Headline(Headline {
                    level: 1,
                    todo_keyword: Some(b("TODO")),
                    todo_type: Some(TodoType::Todo),
                    priority: Some(Priority::Letter('A')),
                    tags: vec![b("t")],
                    commented: false,
                    archived: false,
                    footnote_section: false,
                    raw_title: b("Hi *b*"),
                })),
                0..67,
            ),
            (Event::Start(Tag::Title), 12..18),
            (Event::Text(b("Hi ")), 12..15),
            (Event::Start(Tag::Bold), 15..18),
            (Event::Text(b("b")), 16..17),
            (Event::End(TagEnd::Bold), 15..18),
            (Event::End(TagEnd::Title), 12..18),
            (Event::Start(Tag::Section), 23..67),
            (
                Event::Keyword(Keyword {
                    key: b("NAME"),
                    value: b("p"),
                    optional: None,
                    affiliated: true,
                }),
                23..33,
            ),
            (Event::Start(Tag::Paragraph), 23..67),
            (Event::Text(b("Some")), 33..37),
            (Event::SoftBreak, 37..38),
            (Event::Text(b("text ")), 38..43),
            (Event::Timestamp(ts), 43..63),
            (Event::End(TagEnd::Paragraph), 23..67),
            (Event::End(TagEnd::Section), 23..67),
            (Event::End(TagEnd::Headline), 0..67),
        ];
        let mut actual = events_to_json(src, events).unwrap();
        let mut expected = json!({
          "type": "org-data",
          "children": [{
            "type": "headline", "begin": 0, "end": 67,
            "props": {
              "level": 1, "todo-keyword": "TODO", "todo-type": "todo", "priority": 65,
              "tags": ["t"], "commentedp": false, "archivedp": false,
              "footnote-section-p": false, "raw-value": "Hi *b*",
              "title": [
                {"type": "plain-text", "value": "Hi "},
                {"type": "bold", "begin": 15, "end": 18, "props": {},
                 "children": [{"type": "plain-text", "value": "b"}]}
              ]
            },
            "children": [{
              "type": "section", "begin": 23, "end": 67, "props": {},
              "children": [{
                "type": "paragraph", "begin": 23, "end": 67,
                "props": {"affiliated": [{"key": "NAME", "value": "p", "optional": null}]},
                "children": [
                  {"type": "plain-text", "value": "Some\ntext "},
                  {"type": "timestamp", "begin": 43, "end": 63, "props": {
                    "type": "active", "raw-value": "<2024-01-01 Mon +1w>",
                    "year-start": 2024, "month-start": 1, "day-start": 1,
                    "hour-start": null, "minute-start": null,
                    "year-end": 2024, "month-end": 1, "day-end": 1,
                    "hour-end": null, "minute-end": null,
                    "repeater-type": "cumulate", "repeater-value": 1, "repeater-unit": "week",
                    "warning-type": null, "warning-value": null, "warning-unit": null
                  }, "children": []},
                  {"type": "plain-text", "value": "\n"}
                ]
              }]
            }]
          }]
        });
        // The trailing "\n" above only exists on the expected side: it is
        // removed by normalization.
        normalize(&mut actual);
        normalize(&mut expected);
        assert_eq!(expected, actual);
    }

    #[test]
    fn items_raw_blocks_and_leaves() {
        let src = "- [@c] [X] t :: d\n#+begin_src sh\necho\n#+end_src\n<<<radio>>>\n";
        let events = vec![
            (
                Event::Start(Tag::PlainList {
                    kind: ListKind::Descriptive,
                }),
                0..18,
            ),
            (
                Event::Start(Tag::Item(Item {
                    bullet: b("- "),
                    counter: Some(b("c")),
                    checkbox: Some(Checkbox::On),
                })),
                0..18,
            ),
            (Event::Start(Tag::ItemTag), 11..12),
            (Event::Text(b("t")), 11..12),
            (Event::End(TagEnd::ItemTag), 11..12),
            (Event::Start(Tag::Paragraph), 16..18),
            (Event::Text(b("d")), 16..17),
            (Event::End(TagEnd::Paragraph), 16..18),
            (Event::End(TagEnd::Item), 0..18),
            (Event::End(TagEnd::PlainList(ListKind::Descriptive)), 0..18),
            (
                Event::Start(Tag::SrcBlock {
                    language: Some(b("sh")),
                    switches: None,
                    parameters: None,
                }),
                18..48,
            ),
            (Event::Text(b("echo")), 18..48),
            (Event::End(TagEnd::SrcBlock), 18..48),
            (Event::Start(Tag::Paragraph), 48..60),
            (Event::Start(Tag::RadioTarget), 48..59),
            (Event::Text(b("radio")), 51..56),
            (Event::End(TagEnd::RadioTarget), 48..59),
            (Event::End(TagEnd::Paragraph), 48..60),
            (
                Event::Clock {
                    timestamp: Some(Timestamp {
                        kind: TimestampKind::Inactive,
                        raw: b("[2024-01-01 Mon]"),
                        start: Some(DateTime {
                            year: 2024,
                            month: 1,
                            day: 1,
                            hour: None,
                            minute: None,
                        }),
                        end: None,
                        repeater: None,
                        warning: None,
                    }),
                    duration: None,
                    status: ClockStatus::Running,
                },
                60..60,
            ),
        ];
        let mut actual = events_to_json(src, events).unwrap();
        normalize(&mut actual);
        let expected = json!({
          "type": "org-data",
          "children": [
            {"type": "plain-list", "begin": 0, "end": 18, "props": {"type": "descriptive"}, "children": [
              {"type": "item", "begin": 0, "end": 18,
               "props": {"bullet": "- ", "checkbox": "on", "counter": 3,
                         "tag": [{"type": "plain-text", "value": "t"}]},
               "children": [
                 {"type": "paragraph", "begin": 16, "end": 18, "props": {},
                  "children": [{"type": "plain-text", "value": "d"}]}
               ]}
            ]},
            {"type": "src-block", "begin": 18, "end": 48,
             "props": {"language": "sh", "value": "echo"}, "children": []},
            {"type": "paragraph", "begin": 48, "end": 60, "props": {}, "children": [
              {"type": "radio-target", "begin": 48, "end": 59, "props": {"value": "radio"},
               "children": [{"type": "plain-text", "value": "radio"}]}
            ]},
            {"type": "clock", "begin": 60, "end": 60, "props": {
              "status": "running",
              "value": {"type": "timestamp", "children": [], "props": {
                "type": "inactive", "raw-value": "[2024-01-01 Mon]",
                "year-start": 2024, "month-start": 1, "day-start": 1,
                "year-end": 2024, "month-end": 1, "day-end": 1
              }}
            }, "children": []}
          ]
        });
        assert_eq!(expected, actual);
    }

    #[test]
    fn rejects_malformed_streams() {
        let unbalanced = vec![(Event::Start(Tag::Paragraph), 0..1)];
        assert!(events_to_json("x", unbalanced).is_err());
        let mismatched = vec![
            (Event::Start(Tag::Paragraph), 0..1),
            (Event::End(TagEnd::Bold), 0..1),
        ];
        assert!(events_to_json("x", mismatched).is_err());
        let dangling = vec![(
            Event::Keyword(Keyword {
                key: b("NAME"),
                value: b("x"),
                optional: None,
                affiliated: true,
            }),
            0..9,
        )];
        assert!(events_to_json("#+NAME: x", dangling).is_err());
    }

    #[test]
    fn normalization_rules() {
        let mut v = json!({"type": "org-data", "children": [
          {"type": "paragraph", "begin": 0, "end": 5, "props": {"x": null}, "children": [
            {"type": "plain-text", "value": "a"},
            {"type": "plain-text", "value": "b \n"}
          ]},
          {"type": "src-block", "begin": 5, "end": 9, "props": {"value": "x\n"}, "children": []},
          {"type": "citation-reference", "begin": 0, "end": 1,
           "props": {"key": "k", "prefix": " ", "suffix": " p. 4 "}, "children": []}
        ]});
        normalize(&mut v);
        assert_eq!(
            v,
            json!({"type": "org-data", "children": [
              {"type": "paragraph", "begin": 0, "end": 5, "props": {}, "children": [
                {"type": "plain-text", "value": "ab"}
              ]},
              {"type": "src-block", "begin": 5, "end": 9, "props": {"value": "x"}, "children": []},
              {"type": "citation-reference", "begin": 0, "end": 1,
               "props": {"key": "k", "suffix": "p. 4"}, "children": []}
            ]})
        );
        assert_eq!(item_counter(&Some("12")), json!(12));
        assert_eq!(item_counter(&Some("b")), json!(2));
        assert_eq!(translate_keyword("tblname"), "NAME");
        assert_eq!(translate_keyword("headers"), "HEADER");
    }
}

/// Round trip: Emacs JSON -> synthetic event stream -> `events_to_json`.
///
/// This checks that the converter and `dump.el` agree on every type and
/// property name actually present in the fixtures, independently of the
/// parser: a perfect parser would produce exactly these events.
#[cfg(test)]
mod roundtrip {
    use super::*;
    use pretty_assertions::assert_eq;
    use pullorg::{DateTime, Entity, Repeater, Warning};
    use std::borrow::Cow;

    type Ev = (Event<'static>, Range<usize>);

    fn st(v: &Value) -> Cow<'static, str> {
        Cow::Owned(
            v.as_str()
                .unwrap_or_else(|| panic!("not a string: {v}"))
                .to_string(),
        )
    }

    fn ost(v: &Value) -> Option<Cow<'static, str>> {
        v.as_str().map(|x| Cow::Owned(x.to_string()))
    }

    fn range(n: &Value) -> Range<usize> {
        n["begin"].as_u64().unwrap_or(0) as usize..n["end"].as_u64().unwrap_or(0) as usize
    }

    fn unit(v: &Value) -> TimeUnit {
        match v.as_str().unwrap() {
            "hour" => TimeUnit::Hour,
            "day" => TimeUnit::Day,
            "week" => TimeUnit::Week,
            "month" => TimeUnit::Month,
            "year" => TimeUnit::Year,
            u => panic!("unit {u}"),
        }
    }

    fn timestamp(p: &Value) -> Timestamp<'static> {
        let dt = |suffix: &str| -> Option<DateTime> {
            Some(DateTime {
                year: p[format!("year-{suffix}")].as_u64()? as u16,
                month: p[format!("month-{suffix}")].as_u64()? as u8,
                day: p[format!("day-{suffix}")].as_u64()? as u8,
                hour: p[format!("hour-{suffix}")].as_u64().map(|x| x as u8),
                minute: p[format!("minute-{suffix}")].as_u64().map(|x| x as u8),
            })
        };
        let kind = match p["type"].as_str().unwrap() {
            "active" => TimestampKind::Active,
            "inactive" => TimestampKind::Inactive,
            "active-range" => TimestampKind::ActiveRange,
            "inactive-range" => TimestampKind::InactiveRange,
            "diary" => TimestampKind::Diary,
            k => panic!("timestamp type {k}"),
        };
        let is_range = matches!(
            kind,
            TimestampKind::ActiveRange | TimestampKind::InactiveRange
        );
        Timestamp {
            kind,
            raw: st(&p["raw-value"]),
            start: dt("start"),
            end: if is_range { dt("end") } else { None },
            repeater: p["repeater-type"].as_str().map(|k| Repeater {
                kind: match k {
                    "cumulate" => RepeaterKind::Cumulate,
                    "catch-up" => RepeaterKind::CatchUp,
                    "restart" => RepeaterKind::Restart,
                    k => panic!("repeater {k}"),
                },
                value: p["repeater-value"].as_u64().unwrap() as u32,
                unit: unit(&p["repeater-unit"]),
            }),
            warning: p["warning-type"].as_str().map(|k| Warning {
                kind: match k {
                    "all" => WarningKind::All,
                    "first" => WarningKind::First,
                    k => panic!("warning {k}"),
                },
                value: p["warning-value"].as_u64().unwrap() as u32,
                unit: unit(&p["warning-unit"]),
            }),
        }
    }

    fn opt_ts(v: &Value) -> Option<Timestamp<'static>> {
        (!v.is_null()).then(|| timestamp(&v["props"]))
    }

    fn headline(p: &Value) -> Headline<'static> {
        Headline {
            level: p["level"].as_u64().unwrap() as usize,
            todo_keyword: ost(&p["todo-keyword"]),
            todo_type: p["todo-type"].as_str().map(|t| match t {
                "todo" => TodoType::Todo,
                _ => TodoType::Done,
            }),
            priority: p["priority"].as_u64().map(|v| {
                if v <= 64 {
                    Priority::Numeric(v as u8)
                } else {
                    Priority::Letter(char::from_u32(v as u32).unwrap())
                }
            }),
            tags: p["tags"].as_array().unwrap().iter().map(st).collect(),
            commented: p["commentedp"].as_bool().unwrap(),
            archived: p["archivedp"].as_bool().unwrap(),
            footnote_section: p["footnote-section-p"].as_bool().unwrap(),
            raw_title: st(&p["raw-value"]),
        }
    }

    fn text(value: &str, r: &Range<usize>, out: &mut Vec<Ev>) {
        for (i, line) in value.split('\n').enumerate() {
            if i > 0 {
                out.push((Event::SoftBreak, r.clone()));
            }
            if !line.is_empty() {
                out.push((Event::Text(Cow::Owned(line.to_string())), r.clone()));
            }
        }
    }

    fn nodes(ns: &Value, out: &mut Vec<Ev>) {
        for n in ns.as_array().unwrap() {
            emit(n, out);
        }
    }

    fn container(tag: Tag<'static>, n: &Value, out: &mut Vec<Ev>) {
        let r = range(n);
        let end = tag.to_end();
        out.push((Event::Start(tag), r.clone()));
        nodes(&n["children"], out);
        out.push((Event::End(end), r));
    }

    fn raw(tag: Tag<'static>, n: &Value, out: &mut Vec<Ev>) {
        let r = range(n);
        let end = tag.to_end();
        out.push((Event::Start(tag), r.clone()));
        if let Some(v) = n["props"]["value"].as_str() {
            out.push((Event::Text(Cow::Owned(v.to_string())), r.clone()));
        }
        out.push((Event::End(end), r));
    }

    fn emit(n: &Value, out: &mut Vec<Ev>) {
        let ty = n["type"].as_str().unwrap();
        let p = &n["props"];
        let r = range(n);
        if ty == "plain-text" {
            text(n["value"].as_str().unwrap(), &r, out);
            return;
        }
        if let Some(aff) = p["affiliated"].as_array() {
            for k in aff {
                out.push((
                    Event::Keyword(Keyword {
                        key: st(&k["key"]),
                        value: st(&k["value"]),
                        optional: ost(&k["optional"]),
                        affiliated: true,
                    }),
                    r.clone(),
                ));
            }
        }
        let leaf = |e: Event<'static>, out: &mut Vec<Ev>| out.push((e, r.clone()));
        match ty {
            "headline" | "inlinetask" => {
                let h = headline(p);
                let tag = if ty == "headline" {
                    Tag::Headline(h)
                } else {
                    Tag::Inlinetask(h)
                };
                let end = tag.to_end();
                out.push((Event::Start(tag), r.clone()));
                out.push((Event::Start(Tag::Title), r.clone()));
                nodes(&p["title"], out);
                out.push((Event::End(TagEnd::Title), r.clone()));
                nodes(&n["children"], out);
                out.push((Event::End(end), r));
            }
            "item" => {
                let tag = Tag::Item(Item {
                    bullet: st(&p["bullet"]),
                    counter: p["counter"].as_u64().map(|c| Cow::Owned(c.to_string())),
                    checkbox: p["checkbox"].as_str().map(|c| match c {
                        "on" => Checkbox::On,
                        "off" => Checkbox::Off,
                        _ => Checkbox::Trans,
                    }),
                });
                out.push((Event::Start(tag), r.clone()));
                if p["tag"].is_array() {
                    out.push((Event::Start(Tag::ItemTag), r.clone()));
                    nodes(&p["tag"], out);
                    out.push((Event::End(TagEnd::ItemTag), r.clone()));
                }
                nodes(&n["children"], out);
                out.push((Event::End(TagEnd::Item), r));
            }
            "section" => container(Tag::Section, n, out),
            "paragraph" => container(Tag::Paragraph, n, out),
            "plain-list" => {
                let kind = match p["type"].as_str().unwrap() {
                    "ordered" => ListKind::Ordered,
                    "descriptive" => ListKind::Descriptive,
                    _ => ListKind::Unordered,
                };
                container(Tag::PlainList { kind }, n, out)
            }
            "table" => {
                let el = p["type"] == "table.el";
                let tag = Tag::Table {
                    kind: if el {
                        TableKind::TableEl
                    } else {
                        TableKind::Org
                    },
                    formulas: p["tblfm"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .rev()
                        .map(st)
                        .collect(),
                };
                if el {
                    raw(tag, n, out)
                } else {
                    container(tag, n, out)
                }
            }
            "table-row" => {
                let kind = if p["type"] == "rule" {
                    TableRowKind::Rule
                } else {
                    TableRowKind::Standard
                };
                container(Tag::TableRow { kind }, n, out)
            }
            "table-cell" => container(Tag::TableCell, n, out),
            "center-block" => container(Tag::CenterBlock, n, out),
            "quote-block" => container(Tag::QuoteBlock, n, out),
            "verse-block" => container(Tag::VerseBlock, n, out),
            "special-block" => container(
                Tag::SpecialBlock {
                    name: st(&p["type"]),
                    parameters: ost(&p["parameters"]),
                },
                n,
                out,
            ),
            "src-block" => raw(
                Tag::SrcBlock {
                    language: ost(&p["language"]),
                    switches: ost(&p["switches"]),
                    parameters: ost(&p["parameters"]),
                },
                n,
                out,
            ),
            "example-block" => raw(
                Tag::ExampleBlock {
                    switches: ost(&p["switches"]),
                },
                n,
                out,
            ),
            "export-block" => raw(
                Tag::ExportBlock {
                    backend: st(&p["type"]),
                },
                n,
                out,
            ),
            "comment-block" => raw(Tag::CommentBlock, n, out),
            "drawer" => container(
                Tag::Drawer {
                    name: st(&p["drawer-name"]),
                },
                n,
                out,
            ),
            "property-drawer" => container(Tag::PropertyDrawer, n, out),
            "dynamic-block" => container(
                Tag::DynamicBlock {
                    name: st(&p["block-name"]),
                    parameters: ost(&p["arguments"]),
                },
                n,
                out,
            ),
            "footnote-definition" => container(
                Tag::FootnoteDefinition {
                    label: st(&p["label"]),
                },
                n,
                out,
            ),
            "bold" => container(Tag::Bold, n, out),
            "italic" => container(Tag::Italic, n, out),
            "underline" => container(Tag::Underline, n, out),
            "strike-through" => container(Tag::StrikeThrough, n, out),
            "link" => container(
                Tag::Link(Link {
                    link_type: st(&p["type"]),
                    path: st(&p["path"]),
                    raw_link: st(&p["raw-link"]),
                    format: match p["format"].as_str().unwrap() {
                        "plain" => LinkFormat::Plain,
                        "angle" => LinkFormat::Angle,
                        _ => LinkFormat::Bracket,
                    },
                    search_option: ost(&p["search-option"]),
                    application: ost(&p["application"]),
                }),
                n,
                out,
            ),
            "subscript" | "superscript" => {
                let use_brackets = p["use-brackets-p"].as_bool().unwrap();
                let tag = if ty == "subscript" {
                    Tag::Subscript { use_brackets }
                } else {
                    Tag::Superscript { use_brackets }
                };
                container(tag, n, out)
            }
            "radio-target" => container(Tag::RadioTarget, n, out),
            "footnote-reference" if p["type"] == "inline" => container(
                Tag::FootnoteReference {
                    label: ost(&p["label"]),
                },
                n,
                out,
            ),
            "footnote-reference" => leaf(Event::FootnoteReference(st(&p["label"])), out),
            "citation" => container(
                Tag::Citation {
                    style: ost(&p["style"]),
                    prefix: ost(&p["prefix"]),
                    suffix: ost(&p["suffix"]),
                },
                n,
                out,
            ),
            "citation-reference" => leaf(
                Event::CitationReference {
                    key: st(&p["key"]),
                    prefix: ost(&p["prefix"]),
                    suffix: ost(&p["suffix"]),
                },
                out,
            ),
            "code" => leaf(Event::Code(st(&p["value"])), out),
            "verbatim" => leaf(Event::Verbatim(st(&p["value"])), out),
            "entity" => leaf(
                Event::Entity(Entity {
                    name: Box::leak(p["name"].as_str().unwrap().to_string().into_boxed_str()),
                    latex: "",
                    latex_math: false,
                    html: "",
                    ascii: "",
                    latin1: "",
                    utf8: "",
                    use_brackets: p["use-brackets-p"].as_bool().unwrap(),
                    _marker: std::marker::PhantomData,
                }),
                out,
            ),
            "latex-fragment" => leaf(Event::LatexFragment(st(&p["value"])), out),
            "export-snippet" => leaf(
                Event::ExportSnippet {
                    backend: st(&p["back-end"]),
                    value: st(&p["value"]),
                },
                out,
            ),
            "inline-src-block" => leaf(
                Event::InlineSrcBlock {
                    language: st(&p["language"]),
                    parameters: ost(&p["parameters"]),
                    value: st(&p["value"]),
                },
                out,
            ),
            "inline-babel-call" => leaf(
                Event::InlineBabelCall {
                    call: st(&p["call"]),
                    inside_header: ost(&p["inside-header"]),
                    arguments: ost(&p["arguments"]),
                    end_header: ost(&p["end-header"]),
                },
                out,
            ),
            "babel-call" => leaf(
                Event::BabelCall {
                    call: st(&p["call"]),
                    inside_header: ost(&p["inside-header"]),
                    arguments: ost(&p["arguments"]),
                    end_header: ost(&p["end-header"]),
                },
                out,
            ),
            "macro" => leaf(
                Event::Macro {
                    key: st(&p["key"]),
                    args: p["args"].as_array().unwrap().iter().map(st).collect(),
                },
                out,
            ),
            "target" => leaf(Event::Target(st(&p["value"])), out),
            "statistics-cookie" => leaf(Event::StatisticsCookie(st(&p["value"])), out),
            "timestamp" => leaf(Event::Timestamp(timestamp(p)), out),
            "line-break" => leaf(Event::LineBreak, out),
            "horizontal-rule" => leaf(Event::Rule, out),
            "keyword" => leaf(
                Event::Keyword(Keyword {
                    key: st(&p["key"]),
                    value: st(&p["value"]),
                    optional: None,
                    affiliated: false,
                }),
                out,
            ),
            "comment" => leaf(Event::Comment(st(&p["value"])), out),
            "fixed-width" => leaf(Event::FixedWidth(st(&p["value"])), out),
            "latex-environment" => leaf(Event::LatexEnvironment(st(&p["value"])), out),
            "diary-sexp" => leaf(Event::DiarySexp(st(&p["value"])), out),
            "clock" => leaf(
                Event::Clock {
                    timestamp: opt_ts(&p["value"]),
                    duration: ost(&p["duration"]),
                    status: if p["status"] == "running" {
                        ClockStatus::Running
                    } else {
                        ClockStatus::Closed
                    },
                },
                out,
            ),
            "planning" => leaf(
                Event::Planning {
                    closed: opt_ts(&p["closed"]),
                    scheduled: opt_ts(&p["scheduled"]),
                    deadline: opt_ts(&p["deadline"]),
                },
                out,
            ),
            "node-property" => leaf(
                Event::NodeProperty {
                    key: st(&p["key"]),
                    value: st(&p["value"]),
                },
                out,
            ),
            other => panic!("roundtrip: unhandled type {other}"),
        }
    }

    #[test]
    fn emacs_json_roundtrips_through_events() {
        for name in fixture_names() {
            let mut expected = load_expected(&name).unwrap();
            let src = fs::read_to_string(fixtures_dir().join(format!("{name}.org"))).unwrap();
            let mut events = Vec::new();
            nodes(&expected["children"], &mut events);
            let mut actual =
                events_to_json(&src, events).unwrap_or_else(|e| panic!("fixture {name}: {e}"));
            normalize(&mut expected);
            normalize(&mut actual);
            assert_eq!(expected, actual, "fixture {name}");
        }
    }
}
