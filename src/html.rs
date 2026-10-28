//! HTML renderer.
//!
//! Mirrors `pulldown_cmark::html`: feed any `Iterator<Item = Event>` to
//! [`push_html`], [`write_html_fmt`] or [`write_html_io`].

use std::fmt;
use std::io;

use crate::event::Event;

/// Render events as HTML, appending to `output`.
pub fn push_html<'a, I>(output: &mut String, iter: I)
where
    I: Iterator<Item = Event<'a>>,
{
    let _ = (output, iter);
    todo!("html renderer")
}

/// Render events as HTML into a [`fmt::Write`].
pub fn write_html_fmt<'a, I, W>(writer: W, iter: I) -> fmt::Result
where
    I: Iterator<Item = Event<'a>>,
    W: fmt::Write,
{
    let _ = (writer, iter);
    todo!("html renderer")
}

/// Render events as HTML into an [`io::Write`].
pub fn write_html_io<'a, I, W>(writer: W, iter: I) -> io::Result<()>
where
    I: Iterator<Item = Event<'a>>,
    W: io::Write,
{
    let _ = (writer, iter);
    todo!("html renderer")
}
