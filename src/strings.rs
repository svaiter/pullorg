//! String type used in events.

use std::borrow::Cow;

/// A string that is either borrowed from the input or owned.
///
/// Most events borrow directly from the source document. Owned strings only
/// appear when the parser had to transform the text (for instance when
/// removing the escaping commas from the body of a source block).
pub type CowStr<'a> = Cow<'a, str>;
