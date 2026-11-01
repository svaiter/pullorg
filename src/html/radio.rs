//! Document-local HTML ids for radio targets and their automatic references.

use std::collections::{HashMap, HashSet};

use crate::{Event, Tag, TagEnd};

pub(super) const PREFIX: &str = "org-radio-";

#[derive(Default)]
pub(super) struct Anchors {
    reserved: HashSet<usize>,
    planned: bool,
    raw_html: bool,
    /// Matched target spelling -> (id, has its anchor been emitted?).
    assigned: HashMap<String, (usize, bool)>,
    last: usize,
}

impl Anchors {
    /// Record explicit ids as we stream. On the first radio event the caller
    /// must reserve later ids before writing the first generated id or href.
    pub(super) fn observe(&mut self, event: &Event<'_>) -> bool {
        if self.planned {
            return false;
        }
        let text = match event {
            Event::Start(Tag::RadioTarget { .. }) => return true,
            Event::Start(Tag::Link(l)) if l.link_type == "radio" => return true,
            Event::Target(s) => s,
            Event::NodeProperty { key, value } if key.eq_ignore_ascii_case("CUSTOM_ID") => value,
            Event::Keyword(k) if k.affiliated && matches!(k.key.as_ref(), "NAME" | "ATTR_HTML") => {
                &k.value
            }
            Event::ExportSnippet { backend, value } if backend.eq_ignore_ascii_case("html") => {
                value
            }
            Event::Start(Tag::ExportBlock { backend }) => {
                self.raw_html = backend.eq_ignore_ascii_case("html");
                return false;
            }
            Event::End(TagEnd::ExportBlock) => {
                self.raw_html = false;
                return false;
            }
            Event::Text(s) if self.raw_html => s,
            _ => return false,
        };
        for (pos, _) in text.match_indices(PREFIX) {
            let rest = &text[pos + PREFIX.len()..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(n) = rest[..digits].parse() {
                self.reserved.insert(n);
            }
        }
        false
    }

    pub(super) fn reserve_tail<'a>(
        &mut self,
        events: impl Iterator<Item = Event<'a>>,
    ) -> Vec<Event<'a>> {
        let tail = events
            .inspect(|event| {
                self.observe(event);
            })
            .collect();
        self.planned = true;
        tail
    }

    pub(super) fn number(&mut self, target: &str, definition: bool) -> usize {
        if let Some((id, emitted)) = self.assigned.get_mut(target) {
            if !definition || !*emitted {
                *emitted |= definition;
                return *id;
            }
            // A repeated definition gets its own id. Links keep using the first.
            return self.next_id();
        }
        let id = self.next_id();
        self.assigned.insert(target.to_owned(), (id, definition));
        id
    }

    fn next_id(&mut self) -> usize {
        self.last += 1;
        while self.reserved.contains(&self.last) {
            self.last += 1;
        }
        self.last
    }
}
