//! Regressions for input-dependent stack overflows (issue #4).

use std::fmt::Write;

use pullorg::{html, Event, Options, Parser, Tag, TagEnd, TodoType};

fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

fn blocks(depth: usize, body: &str) -> String {
    let mut text = String::new();
    // Distinct names are needed: Org closes a block at the first matching end.
    for i in 0..depth {
        writeln!(text, "#+begin_b{i}").unwrap();
    }
    text.push_str(body);
    for i in (0..depth).rev() {
        writeln!(text, "#+end_b{i}").unwrap();
    }
    text
}

/// Check nesting and source ranges while collecting events for rendering.
fn checked_events<'a>(parser: Parser<'a>) -> Vec<Event<'a>> {
    let text = parser.source();
    let mut stack = Vec::new();
    let mut events = Vec::new();
    for (event, range) in parser.into_offset_iter() {
        assert!(range.start <= range.end && range.end <= text.len());
        assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
        match &event {
            Event::Start(tag) => stack.push((tag.to_end(), range)),
            Event::End(end) => assert_eq!(stack.pop(), Some((*end, range))),
            _ => {}
        }
        events.push(event);
    }
    assert!(stack.is_empty());
    events
}

#[test]
fn deep_blocks_parse_render_and_drop_on_small_stack() {
    on_small_stack(|| {
        const DEPTH: usize = 4_000;
        let text = blocks(DEPTH, "x\n");
        // Construction and destruction must be safe even without consuming events.
        drop(Parser::new(&text));
        let mut partial = Parser::new(&text);
        for _ in 0..DEPTH {
            assert!(partial.next().is_some());
        }
        drop(partial);

        let events = checked_events(Parser::new(&text));
        let names: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Start(Tag::SpecialBlock { name, .. }) => Some(name.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(names.len(), DEPTH);
        for (i, name) in names.iter().enumerate() {
            assert_eq!(*name, format!("b{i}"));
        }
        let mut rendered = String::new();
        html::push_html(&mut rendered, events.into_iter());
        assert_eq!(rendered.matches("<div class=\"b").count(), DEPTH);
        assert_eq!(rendered.matches("</div>").count(), DEPTH);
        assert!(rendered.contains("<p>x</p>"));
    });
}

#[test]
fn nested_settings_survive_odd_level_reparse() {
    on_small_stack(|| {
        let mut text = String::from("#+STARTUP: odd\n");
        text.push_str(&blocks(256, "#+TODO: WAIT | DONE\n<<<deep>>> deep\n"));
        text.push_str("* WAIT title\n*** DONE child\n");
        let mut options = Options::default();
        options.inlinetask_min_level = Some(15);
        let events = checked_events(Parser::new_ext(&text, options));
        let headlines: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Start(Tag::Headline(h)) => Some((h.level, h.todo_type)),
                _ => None,
            })
            .collect();
        assert_eq!(
            headlines,
            [(1, Some(TodoType::Todo)), (2, Some(TodoType::Done))]
        );
        assert!(events.iter().any(|e| matches!(e,
            Event::Start(Tag::Link(link)) if link.link_type == "radio" && link.path == "deep"
        )));
    });
}

#[test]
fn nested_headlines_and_lists_preserve_every_level() {
    on_small_stack(|| {
        const DEPTH: usize = 256;
        for headlines in [true, false] {
            let mut text = String::new();
            for i in 0..DEPTH {
                if headlines {
                    writeln!(text, "{} h", "*".repeat(i + 1)).unwrap();
                } else {
                    writeln!(text, "{}- item", "  ".repeat(i)).unwrap();
                }
            }
            let events = checked_events(Parser::new(&text));
            let count = events
                .iter()
                .filter(|e| {
                    matches!(e, Event::End(TagEnd::Headline)) && headlines
                        || matches!(e, Event::End(TagEnd::PlainList(_))) && !headlines
                })
                .count();
            assert_eq!(count, DEPTH);
            let mut rendered = String::new();
            html::push_html(&mut rendered, events.into_iter());
            assert!(!rendered.is_empty());
        }
    });
}

#[test]
fn subscripts_and_superscripts_keep_the_existing_depth_boundary() {
    for marker in ['_', '^'] {
        for (open, close) in [('(', ')'), ('{', '}')] {
            for depth in 1..=4 {
                let text = format!(
                    "x{marker}{}!{}",
                    open.to_string().repeat(depth),
                    close.to_string().repeat(depth)
                );
                let scripts = Parser::new_inline(&text)
                    .filter(|e| {
                        matches!(
                            e,
                            Event::Start(Tag::Subscript { .. } | Tag::Superscript { .. })
                        )
                    })
                    .count();
                assert_eq!(scripts, usize::from(depth <= 3), "{text}");
            }
        }
    }
}

#[test]
fn excessive_subscript_nesting_is_rejected_on_small_stack() {
    on_small_stack(|| {
        for marker in ['_', '^'] {
            for (open, close) in [('(', ')'), ('{', '}')] {
                for balanced in [true, false] {
                    let mut text = format!("x{marker}{}!", open.to_string().repeat(40_000));
                    if balanced {
                        text.push_str(&close.to_string().repeat(40_000));
                    }
                    for events in [
                        checked_events(Parser::new(&text)),
                        checked_events(Parser::new_inline(&text)),
                    ] {
                        assert!(!events.iter().any(|e| matches!(
                            e,
                            Event::Start(Tag::Subscript { .. } | Tag::Superscript { .. })
                        )));
                        let plain: String = events
                            .iter()
                            .filter_map(|e| match e {
                                Event::Text(t) => Some(t.as_ref()),
                                _ => None,
                            })
                            .collect();
                        assert_eq!(plain, text);
                        let mut rendered = String::new();
                        html::push_html(&mut rendered, events.into_iter());
                        assert!(rendered.contains(&text));
                    }
                }
            }
        }
    });
}
