//! Element (block level) parsing.
//!
//! This pass turns the whole document into a tree of [`Element`]s. Containers
//! holding objects (paragraphs, verse blocks, table cells, headline titles,
//! item tags) only record the byte range of their content: objects are parsed
//! lazily by [`crate::object`] when the events are pulled.
//!
//! Reference: `org-element.el`, in particular `org-element--parse-elements`,
//! `org-element--current-element` and the `org-element-*-parser` functions.

use std::ops::Range;

use crate::event::{Event, Headline, Item, Keyword, ListKind, TableKind, TableRowKind};
use crate::options::Options;
use crate::strings::CowStr;

/// A parsed element and its children.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Element<'a> {
    pub kind: ElementKind<'a>,
    /// Byte range of the element in the source, *excluding* trailing blank
    /// lines (Emacs' `:begin` to `:end` minus `:post-blank`). For affiliated
    /// keywords, the range starts at the first keyword line.
    pub range: Range<usize>,
    /// Affiliated keywords attached to this element, in source order.
    pub affiliated: Vec<(Keyword<'a>, Range<usize>)>,
    /// Child elements (for greater elements).
    pub children: Vec<Element<'a>>,
}

/// The kind of an element.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ElementKind<'a> {
    /// `title` is the range of the raw title to parse as objects.
    Headline {
        data: Headline<'a>,
        title: Range<usize>,
    },
    Inlinetask {
        data: Headline<'a>,
        title: Range<usize>,
    },
    Section,
    /// `content` is the range to parse as objects.
    Paragraph {
        content: Range<usize>,
    },
    PlainList {
        kind: ListKind,
    },
    /// `tag` is the range of the item tag to parse as objects.
    Item {
        data: Item<'a>,
        tag: Option<Range<usize>>,
    },
    /// `value` is the raw text of a `table.el` table (`None` for Org tables).
    Table {
        kind: TableKind,
        formulas: Vec<CowStr<'a>>,
        value: Option<CowStr<'a>>,
    },
    TableRow {
        kind: TableRowKind,
    },
    TableCell {
        content: Range<usize>,
    },
    CenterBlock,
    QuoteBlock,
    VerseBlock {
        content: Range<usize>,
    },
    SpecialBlock {
        name: CowStr<'a>,
        parameters: Option<CowStr<'a>>,
    },
    /// `value` is the unescaped block body (trailing newline removed).
    SrcBlock {
        language: Option<CowStr<'a>>,
        switches: Option<CowStr<'a>>,
        parameters: Option<CowStr<'a>>,
        value: CowStr<'a>,
    },
    ExampleBlock {
        switches: Option<CowStr<'a>>,
        value: CowStr<'a>,
    },
    ExportBlock {
        backend: CowStr<'a>,
        value: CowStr<'a>,
    },
    CommentBlock {
        value: CowStr<'a>,
    },
    Drawer {
        name: CowStr<'a>,
    },
    PropertyDrawer,
    DynamicBlock {
        name: CowStr<'a>,
        parameters: Option<CowStr<'a>>,
    },
    FootnoteDefinition {
        label: CowStr<'a>,
    },
    /// A leaf element, already in event form: `Keyword`, `Comment`,
    /// `FixedWidth`, `Rule`, `LatexEnvironment`, `BabelCall`, `DiarySexp`,
    /// `Clock`, `Planning`, `NodeProperty`.
    Leaf(Event<'a>),
}

/// Document-wide settings gathered from in-buffer keywords before parsing,
/// merged with the user [`Options`].
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Settings<'a> {
    pub todo_keywords: Vec<CowStr<'a>>,
    pub done_keywords: Vec<CowStr<'a>>,
    pub sub_superscripts: crate::options::SubSuperscripts,
    pub link_abbreviations: Vec<(CowStr<'a>, CowStr<'a>)>,
    /// Radio targets (`<<<text>>>`) found in the document, used to turn
    /// matching plain text into radio links.
    pub radio_targets: Vec<CowStr<'a>>,
}

/// The parsed document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Document<'a> {
    pub elements: Vec<Element<'a>>,
    pub settings: Settings<'a>,
}

/// Parse the whole document into an element tree.
pub(crate) fn parse_document<'a>(text: &'a str, options: &Options) -> Document<'a> {
    let _ = (text, options);
    todo!("element parser")
}
