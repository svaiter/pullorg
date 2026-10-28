//! Character classes, mirroring the syntax table Emacs uses in an Org buffer
//! (`org-mode-syntax-table`, derived from `text-mode`) and the regexp
//! character classes used by `org-element.el`.
//!
//! The ASCII part is exact (checked against `char-syntax` in an Org buffer).
//! For non-ASCII characters Emacs relies on Unicode tables; we approximate
//! them with the Unicode properties available in `core`.

/// Whitespace syntax (`\s-`, rx `space`, `[[:space:]]`).
/// Exactly the characters with whitespace syntax in an Org buffer (checked
/// against Emacs for every code point below U+30000). Note that the zero
/// width space U+200B is one of them, which is why it can separate emphasis
/// markers from adjacent text.
pub(crate) fn is_ws(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '\r' | '\x0c' | '\u{a0}' | '\u{2000}'
            ..='\u{200b}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

/// Non-ASCII characters which do not have word syntax (punctuation and
/// symbols). Approximation of Emacs' `characters.el`.
fn non_ascii_non_word(c: char) -> bool {
    if c.is_alphanumeric() {
        return false;
    }
    matches!(
        c as u32,
        0xA0..=0xBF
            | 0xD7
            | 0xF7
            | 0x2000..=0x2BFF
            | 0x2E00..=0x2E7F
            | 0x3000..=0x303F
            | 0xFE10..=0xFE6F
            | 0xFF00..=0xFF20
            | 0xFF3B..=0xFF40
            | 0xFF5B..=0xFF65
    )
}

/// Word syntax (`\w`, `[[:word:]]`, used by `\<`). In an Org buffer, `$`, `%`
/// and `'` have word syntax.
pub(crate) fn is_word(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || matches!(c, '$' | '%' | '\'')
    } else {
        !is_ws(c) && !non_ascii_non_word(c)
    }
}

/// `[[:alnum:]]`.
pub(crate) fn is_alnum(c: char) -> bool {
    c.is_alphanumeric()
}

/// `[[:alpha:]]` (rx `letter`).
pub(crate) fn is_alpha(c: char) -> bool {
    c.is_alphabetic()
}

/// `[[:punct:]]`: ASCII punctuation, or any non-ASCII character without word
/// syntax.
pub(crate) fn is_punct(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_punctuation()
    } else {
        !is_word(c) && !is_ws(c)
    }
}

/// Characters after which a line may be broken (category `|` in Emacs),
/// mostly CJK.
pub(crate) fn is_line_breakable(c: char) -> bool {
    matches!(
        c as u32,
        0x2E80..=0x2FFF
            | 0x3000..=0x303F
            | 0x3040..=0x30FF
            | 0x3100..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x20000..=0x2FFFF
    )
}

/// Characters allowed right after the closing `$` of a `$...$` fragment:
/// punctuation, whitespace, open/close parenthesis or string quote syntax,
/// and `'`.
pub(crate) fn latex_dollar_post(c: char) -> bool {
    if c.is_ascii() {
        matches!(
            c,
            '!' | '#'
                | ','
                | '.'
                | ':'
                | ';'
                | '?'
                | '@'
                | '^'
                | '`'
                | '('
                | '['
                | '{'
                | '<'
                | ')'
                | ']'
                | '}'
                | '>'
                | '"'
                | '\''
        ) || is_ws(c)
    } else {
        !is_word(c)
    }
}

/// Case-insensitive character comparison, as done by Emacs with
/// `case-fold-search` bound to `t`.
pub(crate) fn eq_fold(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// ASCII case-insensitive prefix test.
pub(crate) fn starts_with_ci(hay: &str, needle: &str) -> bool {
    hay.len() >= needle.len()
        && hay.as_bytes()[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
}

/// Whether `c` belongs to the Latin script (`char-script-table`), for word
/// boundaries. ASCII is Latin.
pub(crate) fn is_latin_script(c: char) -> bool {
    matches!(
        c as u32,
        0..=0x7F
            | 0xAA
            | 0xB2
            | 0xB3
            | 0xB5
            | 0xB9
            | 0xBA
            | 0xBC..=0xBE
            | 0xC0..=0x24F
            | 0x250..=0x2AF
            | 0x1D00..=0x1DBF
            | 0x1E00..=0x1EFF
            | 0x2C60..=0x2C7F
            | 0xA720..=0xA7FF
            | 0xAB30..=0xAB6F
            | 0xFB00..=0xFB06
    )
}
