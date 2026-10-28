//! The pull parser.

use std::collections::VecDeque;
use std::ops::Range;

use crate::element::{self, Element, ElementKind, Settings};
use crate::event::{Event, Tag, TagEnd};
use crate::object::{self, Restriction};
use crate::options::Options;

/// Arena node, flattened from the element tree.
struct Node<'a> {
    element: Element<'a>,
    children: Vec<usize>,
}

/// What the walker is in the middle of emitting.
enum Frame {
    /// Emitting the children of node `node`, next child index `next`.
    Children { node: usize, next: usize },
    /// Emit the `End` of node `node`.
    End { node: usize },
}

/// A pull parser for Org documents.
///
/// Implements `Iterator<Item = Event>`. Use [`Parser::into_offset_iter`] to
/// also get the byte range of each event.
///
/// ```
/// use pulldown_org::{Parser, Event, Tag};
///
/// let parser = Parser::new("* Hello\nSome *bold* text.");
/// let events: Vec<_> = parser.collect();
/// assert!(matches!(events[0], Event::Start(Tag::Headline(_))));
/// ```
pub struct Parser<'a> {
    text: &'a str,
    nodes: Vec<Node<'a>>,
    settings: Settings<'a>,
    roots: Vec<usize>,
    stack: Vec<Frame>,
    queue: VecDeque<(Event<'a>, Range<usize>)>,
    root_next: usize,
}

impl<'a> Parser<'a> {
    /// Create a parser with the default [`Options`].
    pub fn new(text: &'a str) -> Self {
        Self::new_ext(text, Options::default())
    }

    /// Create a parser with the given [`Options`].
    pub fn new_ext(text: &'a str, options: Options) -> Self {
        let document = element::parse_document(text, &options);
        let mut nodes = Vec::new();
        let roots = document
            .elements
            .into_iter()
            .map(|e| flatten(e, &mut nodes))
            .collect();
        Parser {
            text,
            nodes,
            settings: document.settings,
            roots,
            stack: Vec::new(),
            queue: VecDeque::new(),
            root_next: 0,
        }
    }

    /// Consume the parser and return an iterator yielding each event together
    /// with its byte range in the source text.
    pub fn into_offset_iter(self) -> OffsetIter<'a> {
        OffsetIter { parser: self }
    }

    /// The source text.
    pub fn source(&self) -> &'a str {
        self.text
    }

    fn next_with_range(&mut self) -> Option<(Event<'a>, Range<usize>)> {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return Some(item);
            }
            match self.stack.pop() {
                None => {
                    if self.root_next < self.roots.len() {
                        let node = self.roots[self.root_next];
                        self.root_next += 1;
                        self.emit_start(node);
                    } else {
                        return None;
                    }
                }
                Some(Frame::End { node }) => {
                    let n = &self.nodes[node];
                    if let Some(end) = end_of(&n.element.kind) {
                        self.queue
                            .push_back((Event::End(end), n.element.range.clone()));
                    }
                }
                Some(Frame::Children { node, next }) => {
                    let children = &self.nodes[node].children;
                    if next < children.len() {
                        let child = children[next];
                        self.stack.push(Frame::Children {
                            node,
                            next: next + 1,
                        });
                        self.emit_start(child);
                    } else {
                        self.stack.push(Frame::End { node });
                    }
                }
            }
        }
    }

    /// Queue the start of `node` (affiliated keywords, start tag or leaf
    /// event, object content) and push the frames for its children/end.
    fn emit_start(&mut self, node: usize) {
        let text = self.text;
        let n = &self.nodes[node];
        let range = n.element.range.clone();
        for (kw, kw_range) in &n.element.affiliated {
            self.queue
                .push_back((Event::Keyword(kw.clone()), kw_range.clone()));
        }
        match &n.element.kind {
            ElementKind::Leaf(event) => {
                self.queue.push_back((event.clone(), range));
                return;
            }
            ElementKind::Headline { data, title } => {
                self.queue
                    .push_back((Event::Start(Tag::Headline(data.clone())), range.clone()));
                self.queue
                    .push_back((Event::Start(Tag::Title), title.clone()));
                let mut objs = Vec::new();
                object::parse_objects(
                    text,
                    title.clone(),
                    Restriction::HeadlineTitle,
                    &self.settings,
                    &mut objs,
                );
                self.queue.extend(objs);
                self.queue
                    .push_back((Event::End(TagEnd::Title), title.clone()));
            }
            ElementKind::Inlinetask { data, title } => {
                self.queue
                    .push_back((Event::Start(Tag::Inlinetask(data.clone())), range.clone()));
                self.queue
                    .push_back((Event::Start(Tag::Title), title.clone()));
                let mut objs = Vec::new();
                object::parse_objects(
                    text,
                    title.clone(),
                    Restriction::HeadlineTitle,
                    &self.settings,
                    &mut objs,
                );
                self.queue.extend(objs);
                self.queue
                    .push_back((Event::End(TagEnd::Title), title.clone()));
            }
            ElementKind::Section => {
                self.queue.push_back((Event::Start(Tag::Section), range));
            }
            ElementKind::Paragraph { content } => {
                self.queue.push_back((Event::Start(Tag::Paragraph), range));
                let mut objs = Vec::new();
                object::parse_objects(
                    text,
                    content.clone(),
                    Restriction::Standard,
                    &self.settings,
                    &mut objs,
                );
                self.queue.extend(objs);
            }
            ElementKind::PlainList { kind } => {
                self.queue
                    .push_back((Event::Start(Tag::PlainList { kind: *kind }), range));
            }
            ElementKind::Item { data, tag } => {
                self.queue
                    .push_back((Event::Start(Tag::Item(data.clone())), range));
                if let Some(tag) = tag {
                    self.queue
                        .push_back((Event::Start(Tag::ItemTag), tag.clone()));
                    let mut objs = Vec::new();
                    object::parse_objects(
                        text,
                        tag.clone(),
                        Restriction::ItemTag,
                        &self.settings,
                        &mut objs,
                    );
                    self.queue.extend(objs);
                    self.queue
                        .push_back((Event::End(TagEnd::ItemTag), tag.clone()));
                }
            }
            ElementKind::Table {
                kind,
                formulas,
                value,
            } => {
                self.queue.push_back((
                    Event::Start(Tag::Table {
                        kind: *kind,
                        formulas: formulas.clone(),
                    }),
                    range.clone(),
                ));
                if let Some(value) = value {
                    self.queue.push_back((Event::Text(value.clone()), range));
                }
            }
            ElementKind::TableRow { kind } => {
                self.queue
                    .push_back((Event::Start(Tag::TableRow { kind: *kind }), range));
            }
            ElementKind::TableCell { content } => {
                self.queue.push_back((Event::Start(Tag::TableCell), range));
                let mut objs = Vec::new();
                object::parse_objects(
                    text,
                    content.clone(),
                    Restriction::TableCell,
                    &self.settings,
                    &mut objs,
                );
                self.queue.extend(objs);
            }
            ElementKind::CenterBlock => {
                self.queue
                    .push_back((Event::Start(Tag::CenterBlock), range));
            }
            ElementKind::QuoteBlock => {
                self.queue.push_back((Event::Start(Tag::QuoteBlock), range));
            }
            ElementKind::VerseBlock { content } => {
                self.queue.push_back((Event::Start(Tag::VerseBlock), range));
                let mut objs = Vec::new();
                object::parse_objects(
                    text,
                    content.clone(),
                    Restriction::Standard,
                    &self.settings,
                    &mut objs,
                );
                self.queue.extend(objs);
            }
            ElementKind::SpecialBlock { name, parameters } => {
                self.queue.push_back((
                    Event::Start(Tag::SpecialBlock {
                        name: name.clone(),
                        parameters: parameters.clone(),
                    }),
                    range,
                ));
            }
            ElementKind::SrcBlock {
                language,
                switches,
                parameters,
                value,
            } => {
                self.queue.push_back((
                    Event::Start(Tag::SrcBlock {
                        language: language.clone(),
                        switches: switches.clone(),
                        parameters: parameters.clone(),
                    }),
                    range.clone(),
                ));
                self.queue.push_back((Event::Text(value.clone()), range));
            }
            ElementKind::ExampleBlock { switches, value } => {
                self.queue.push_back((
                    Event::Start(Tag::ExampleBlock {
                        switches: switches.clone(),
                    }),
                    range.clone(),
                ));
                self.queue.push_back((Event::Text(value.clone()), range));
            }
            ElementKind::ExportBlock { backend, value } => {
                self.queue.push_back((
                    Event::Start(Tag::ExportBlock {
                        backend: backend.clone(),
                    }),
                    range.clone(),
                ));
                self.queue.push_back((Event::Text(value.clone()), range));
            }
            ElementKind::CommentBlock { value } => {
                self.queue
                    .push_back((Event::Start(Tag::CommentBlock), range.clone()));
                self.queue.push_back((Event::Text(value.clone()), range));
            }
            ElementKind::Drawer { name } => {
                self.queue
                    .push_back((Event::Start(Tag::Drawer { name: name.clone() }), range));
            }
            ElementKind::PropertyDrawer => {
                self.queue
                    .push_back((Event::Start(Tag::PropertyDrawer), range));
            }
            ElementKind::DynamicBlock { name, parameters } => {
                self.queue.push_back((
                    Event::Start(Tag::DynamicBlock {
                        name: name.clone(),
                        parameters: parameters.clone(),
                    }),
                    range,
                ));
            }
            ElementKind::FootnoteDefinition { label } => {
                self.queue.push_back((
                    Event::Start(Tag::FootnoteDefinition {
                        label: label.clone(),
                    }),
                    range,
                ));
            }
        }
        self.stack.push(Frame::Children { node, next: 0 });
    }
}

fn end_of(kind: &ElementKind<'_>) -> Option<TagEnd> {
    Some(match kind {
        ElementKind::Leaf(_) => return None,
        ElementKind::Headline { .. } => TagEnd::Headline,
        ElementKind::Inlinetask { .. } => TagEnd::Inlinetask,
        ElementKind::Section => TagEnd::Section,
        ElementKind::Paragraph { .. } => TagEnd::Paragraph,
        ElementKind::PlainList { kind } => TagEnd::PlainList(*kind),
        ElementKind::Item { .. } => TagEnd::Item,
        ElementKind::Table { .. } => TagEnd::Table,
        ElementKind::TableRow { .. } => TagEnd::TableRow,
        ElementKind::TableCell { .. } => TagEnd::TableCell,
        ElementKind::CenterBlock => TagEnd::CenterBlock,
        ElementKind::QuoteBlock => TagEnd::QuoteBlock,
        ElementKind::VerseBlock { .. } => TagEnd::VerseBlock,
        ElementKind::SpecialBlock { .. } => TagEnd::SpecialBlock,
        ElementKind::SrcBlock { .. } => TagEnd::SrcBlock,
        ElementKind::ExampleBlock { .. } => TagEnd::ExampleBlock,
        ElementKind::ExportBlock { .. } => TagEnd::ExportBlock,
        ElementKind::CommentBlock { .. } => TagEnd::CommentBlock,
        ElementKind::Drawer { .. } => TagEnd::Drawer,
        ElementKind::PropertyDrawer => TagEnd::PropertyDrawer,
        ElementKind::DynamicBlock { .. } => TagEnd::DynamicBlock,
        ElementKind::FootnoteDefinition { .. } => TagEnd::FootnoteDefinition,
    })
}

fn flatten<'a>(mut element: Element<'a>, nodes: &mut Vec<Node<'a>>) -> usize {
    let children = std::mem::take(&mut element.children);
    let idx = nodes.len();
    nodes.push(Node {
        element,
        children: Vec::new(),
    });
    let child_idx: Vec<usize> = children.into_iter().map(|c| flatten(c, nodes)).collect();
    nodes[idx].children = child_idx;
    idx
}

impl<'a> Iterator for Parser<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        self.next_with_range().map(|(event, _)| event)
    }
}

/// Iterator over events and their byte ranges, see
/// [`Parser::into_offset_iter`].
pub struct OffsetIter<'a> {
    parser: Parser<'a>,
}

impl<'a> Iterator for OffsetIter<'a> {
    type Item = (Event<'a>, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        self.parser.next_with_range()
    }
}
