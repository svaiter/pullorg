//! A pull parser for [Org-mode], in the style of [pulldown-cmark].
//!
//! ```
//! use pulldown_org::{html, Parser};
//!
//! let source = "* A headline\nSome /italic/ text.";
//! let parser = Parser::new(source);
//! let mut out = String::new();
//! html::push_html(&mut out, parser);
//! ```
//!
//! The parser produces a flat stream of [`Event`]s. Containers are delimited
//! by [`Event::Start`] and [`Event::End`]; see [`Tag`] for the list of
//! containers. The vocabulary follows the [Org syntax] document and the
//! behaviour follows `org-element.el`, the reference implementation shipped
//! with Emacs.
//!
//! [Org-mode]: https://orgmode.org
//! [pulldown-cmark]: https://docs.rs/pulldown-cmark
//! [Org syntax]: https://orgmode.org/worg/org-syntax.html

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod element;
mod entities;
mod event;
pub mod html;
mod object;
mod options;
mod parser;
mod strings;

pub use event::*;
pub use options::{Options, SubSuperscripts};
pub use parser::{OffsetIter, Parser};
pub use strings::CowStr;
