//! Object (inline) parsing.
//!
//! Reference: `org-element.el`, in particular `org-element--parse-objects`,
//! `org-element--object-lex`, `org-element-object-restrictions` and the
//! `org-element-*-parser` functions for objects.

use std::ops::Range;

use crate::element::Settings;
use crate::event::Event;

/// Which objects are allowed in a given container. Mirrors
/// `org-element-object-restrictions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Restriction {
    /// Paragraph, verse block: the full "standard set".
    Standard,
    /// Headline and inline task titles: standard set without line breaks.
    HeadlineTitle,
    /// Link description: "minimal set" plus export snippets, inline babel
    /// calls, inline src blocks, macros and statistics cookies.
    LinkDescription,
    /// Table cell.
    TableCell,
    /// Item tag.
    ItemTag,
    /// Affiliated keyword values such as `#+CAPTION:`.
    Keyword,
    /// Inside markup objects (bold, italic, ...): standard set.
    Markup,
    /// Inside subscript / superscript.
    SubSuperscript,
    /// Inside a radio target.
    RadioTarget,
    /// Inside a footnote reference's inline definition.
    FootnoteReference,
}

/// Parse the objects in `text[range]`, appending `(event, range)` pairs to
/// `out`. Ranges are absolute byte offsets in `text`.
///
/// Newlines are emitted as [`Event::SoftBreak`]; text is otherwise emitted as
/// borrowed [`Event::Text`] runs.
pub(crate) fn parse_objects<'a>(
    text: &'a str,
    range: Range<usize>,
    restriction: Restriction,
    settings: &Settings<'a>,
    out: &mut Vec<(Event<'a>, Range<usize>)>,
) {
    let _ = (text, range, restriction, settings, out);
    todo!("object parser")
}
