//! Resolve footnote references before rendering the body. Discovery indexes
//! definitions in source order. Numbering follows references depth-first.

use std::collections::HashMap;

use super::{omit, HInfo};
use crate::{Event, Tag, TagEnd};

pub(super) fn is_footnote(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::FootnoteReference(_)
            | Event::Start(Tag::FootnoteReference { .. } | Tag::FootnoteDefinition { .. })
    )
}

#[derive(Default)]
struct Note {
    definition: Option<usize>,
    number: Option<usize>,
    references: usize,
}

#[derive(Default)]
struct Graph<'a> {
    labels: HashMap<&'a str, usize>,
    notes: Vec<Note>,
    /// For each definition, the reference occurrences in its own contents.
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    /// Source-order references, each pointing to a note.
    references: Vec<usize>,
    open: Vec<usize>,
}

impl<'a> Graph<'a> {
    fn note(&mut self, label: Option<&'a str>) -> usize {
        if let Some(&id) = label.and_then(|l| self.labels.get(l)) {
            return id;
        }
        let id = self.notes.len();
        self.notes.push(Note::default());
        if let Some(label) = label {
            self.labels.insert(label, id);
        }
        id
    }

    fn reference(&mut self, note: usize, visible: bool) {
        let id = self.references.len();
        self.references.push(note);
        if let Some(&def) = self.open.last() {
            self.children[def].push(id);
        } else if visible {
            self.roots.push(id);
        }
    }

    fn definition(&mut self, note: usize) {
        let id = self.children.len();
        self.children.push(Vec::new());
        self.notes[note].definition = Some(id);
        self.open.push(id);
    }

    fn resolve(mut self) -> Plan {
        let mut links = vec![None; self.references.len()];
        let mut destinations = vec![None; self.children.len()];
        let mut bodies = Vec::new();
        // A note gets its number on first visit, before its children are
        // scheduled. Cycles terminate, and long chains use no call stack.
        self.roots.reverse();
        while let Some(reference) = self.roots.pop() {
            let note = &mut self.notes[self.references[reference]];
            let number = *note.number.get_or_insert_with(|| {
                bodies.push(String::new());
                let number = bodies.len();
                if let Some(def) = note.definition {
                    destinations[def] = Some(number);
                    self.roots.extend(self.children[def].iter().rev());
                }
                number
            });
            note.references += 1;
            links[reference] = Some((number, note.references));
        }
        Plan {
            links: links.into_iter(),
            destinations: destinations.into_iter(),
            open: Vec::new(),
            bodies,
        }
    }
}

/// Both iterators are consumed in source order by the renderer. An empty
/// slot denotes a reference or definition that contributes no HTML.
pub(super) struct Plan {
    links: std::vec::IntoIter<Option<(usize, usize)>>,
    destinations: std::vec::IntoIter<Option<usize>>,
    open: Vec<Option<usize>>,
    pub(super) bodies: Vec<String>,
}

impl Plan {
    pub(super) fn discover<'s, 'a: 's>(
        events: impl Iterator<Item = &'s Event<'a>>,
        headings: &[HInfo],
        mut skip: usize,
        mut register_radio: impl FnMut(&str),
    ) -> Self {
        let mut graph = Graph::default();
        // Targets inside definitions become visible only once the reference
        // graph has selected the reachable definitions.
        let mut radios_in_defs = Vec::new();
        // (archived, inside a muted footnote heading). The first footnote
        // can appear inside a heading whose start has already been rendered.
        let mut heads = Vec::new();
        let mut muted = false;
        for h in headings {
            muted |= h.muted;
            heads.push((h.archived, muted));
        }
        for event in events {
            if skip != 0 {
                match event {
                    Event::Start(_) => skip += 1,
                    Event::End(_) => skip -= 1,
                    _ => {}
                }
                continue;
            }
            let (archived, muted) = heads.last().copied().unwrap_or_default();
            match event {
                Event::Start(tag) if omit(tag, archived) => skip = 1,
                Event::Start(Tag::Headline(h)) => {
                    heads.push((h.archived, muted || h.footnote_section))
                }
                Event::Start(Tag::Inlinetask(_)) => heads.push((false, muted)),
                Event::End(TagEnd::Headline | TagEnd::Inlinetask) => {
                    heads.pop();
                }
                Event::Start(Tag::RadioTarget { value }) => {
                    if let Some(&def) = graph.open.last() {
                        radios_in_defs.push((def, value));
                    } else if !muted {
                        register_radio(value);
                    }
                }
                Event::FootnoteReference(label) => {
                    let note = graph.note(Some(label));
                    graph.reference(note, !muted);
                }
                Event::Start(Tag::FootnoteReference { label }) => {
                    let note = graph.note(label.as_deref());
                    graph.reference(note, !muted);
                    graph.definition(note);
                }
                Event::Start(Tag::FootnoteDefinition { label }) => {
                    let note = graph.note(Some(label));
                    graph.definition(note);
                }
                Event::End(TagEnd::FootnoteDefinition | TagEnd::FootnoteReference) => {
                    graph.open.pop();
                }
                _ => {}
            }
        }
        let plan = graph.resolve();
        for (def, value) in radios_in_defs {
            if plan.destinations.as_slice()[def].is_some() {
                register_radio(value);
            }
        }
        plan
    }

    pub(super) fn reference(&mut self) -> Option<(usize, usize)> {
        self.links.next().flatten()
    }

    pub(super) fn begin_definition(&mut self) -> bool {
        let destination = self.destinations.next().flatten();
        self.open.push(destination);
        destination.is_some()
    }

    pub(super) fn end_definition(&mut self, contents: String) {
        if let Some(Some(number)) = self.open.pop() {
            self.bodies[number - 1] = contents;
        }
    }

    pub(super) fn visible_definition(&self) -> Option<bool> {
        self.open.last().map(Option::is_some)
    }
}
