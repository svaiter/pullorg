//! Parser options.

/// Options controlling the parser.
///
/// Create with [`Options::default`] and adjust the fields you need. In-buffer
/// settings (`#+TODO:`, `#+OPTIONS:`, `#+LINK:`, ...) found in the document
/// override these defaults, exactly as Emacs does.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// TODO keywords denoting open tasks. Default: `["TODO"]`.
    pub todo_keywords: Vec<String>,
    /// TODO keywords denoting closed tasks. Default: `["DONE"]`.
    pub done_keywords: Vec<String>,
    /// How `a_b` / `a^b` are interpreted.
    pub sub_superscripts: SubSuperscripts,
    /// Minimum number of stars for a headline to be parsed as an inline
    /// task. `None` disables inline tasks (the default, as in a stock Emacs
    /// without `org-inlinetask` loaded).
    pub inlinetask_min_level: Option<usize>,
    /// Title of the footnote section headline. Default: `"Footnotes"`.
    pub footnote_section: Option<String>,
    /// Link abbreviations (`#+LINK: key url`), as `(key, replacement)`.
    pub link_abbreviations: Vec<(String, String)>,
    /// Additional link types, as registered in Emacs with
    /// `org-link-set-parameters` (e.g. `["doi", "media"]` for a site that
    /// writes `[[media:key][text]]`).
    ///
    /// The built-in types of a stock Emacs (`https`, `file`, `id`, `doi`,
    /// ...) are always recognised. With `media` registered,
    /// `[[media:key]]` is a [`Link`](crate::Link) of type `media` and path
    /// `key` instead of a `fuzzy` link with path `media:key`, and, as in
    /// Emacs, a plain `media:key` or `<media:key>` in text is a link too.
    /// Types are matched case-insensitively.
    pub link_types: Vec<String>,
}

/// Interpretation of `_` and `^` (the `^:` export option and
/// `org-use-sub-superscripts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SubSuperscripts {
    /// `a_b` and `a_{b}` are both subscripts (`^:t`).
    #[default]
    Always,
    /// Only `a_{b}` is a subscript (`^:{}`).
    Braces,
    /// Never parse sub/superscripts (`^:nil`).
    Never,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            todo_keywords: vec!["TODO".to_string()],
            done_keywords: vec!["DONE".to_string()],
            sub_superscripts: SubSuperscripts::Always,
            inlinetask_min_level: None,
            footnote_section: Some("Footnotes".to_string()),
            link_abbreviations: Vec::new(),
            link_types: Vec::new(),
        }
    }
}
