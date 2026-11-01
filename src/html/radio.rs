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
    assigned: HashMap<String, (Option<usize>, bool)>,
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

    /// Only anchors that can actually appear in the rendered document can
    /// receive a number. A link can precede its visible definition.
    pub(super) fn register(&mut self, target: &str) {
        if !self.assigned.contains_key(target) {
            self.assigned.insert(target.to_owned(), (None, false));
        }
    }

    pub(super) fn number(&mut self, target: &str, definition: bool) -> Option<usize> {
        let (number, emitted) = self.assigned.get_mut(target)?;
        let mut next_id = || {
            self.last += 1;
            while self.reserved.contains(&self.last) {
                self.last += 1;
            }
            self.last
        };
        let number = *number.get_or_insert_with(&mut next_id);
        if definition && std::mem::replace(emitted, true) {
            // Repeated definitions get distinct ids; links use the first.
            Some(next_id())
        } else {
            Some(number)
        }
    }
}
