//! The event types produced by the parser.
//!
//! The design follows [pulldown-cmark]: a document is a flat stream of
//! [`Event`]s. Containers (elements or objects that may hold other content)
//! are delimited by [`Event::Start`] / [`Event::End`] pairs, everything else
//! is a leaf event.
//!
//! The vocabulary follows the [Org syntax] document and the reference
//! implementation in Emacs (`org-element.el`): *elements* are the block level
//! constructs (headlines, paragraphs, lists, blocks, ...) and *objects* are
//! the inline constructs (markup, links, timestamps, ...).
//!
//! [pulldown-cmark]: https://docs.rs/pulldown-cmark
//! [Org syntax]: https://orgmode.org/worg/org-syntax.html

use crate::strings::CowStr;

/// A parsing event.
///
/// Containers produce a `Start(Tag)` event, followed by their content, then an
/// `End(TagEnd)` event. All other variants are leaves.
#[derive(Clone, Debug, PartialEq)]
pub enum Event<'a> {
    /// Start of a container (element or object).
    Start(Tag<'a>),
    /// End of a container.
    End(TagEnd),

    /// Plain text.
    ///
    /// Text never spans a line ending inside a paragraph; a [`Event::SoftBreak`]
    /// is emitted instead. Inside raw containers such as source or example
    /// blocks, the text is emitted verbatim (including newlines and the
    /// indentation as written, like `org-element`'s `:value`), after
    /// removing the `,` escaping of lines starting with `*` or `#+`.
    ///
    /// The whitespace following an object (`org-element`'s `:post-blank`)
    /// is not part of the object: it starts the `Text` that follows it, so
    /// `*a* b` gives `Start(Bold)`, `Text("a")`, `End(Bold)`, `Text(" b")`.
    Text(CowStr<'a>),
    /// `~code~` markup.
    Code(CowStr<'a>),
    /// `=verbatim=` markup.
    Verbatim(CowStr<'a>),
    /// An entity such as `\alpha` or `\nbsp{}`.
    Entity(Entity<'a>),
    /// A LaTeX fragment: `$x$`, `\(x\)`, `\[x\]`, `$$x$$` or `\command{...}`.
    ///
    /// The value is the full raw fragment including delimiters.
    LatexFragment(CowStr<'a>),
    /// An export snippet `@@backend:value@@`.
    ExportSnippet {
        /// The backend name (`html`, `latex`, ...).
        backend: CowStr<'a>,
        /// The raw value to insert when exporting to this backend.
        value: CowStr<'a>,
    },
    /// An inline source block `src_lang[params]{body}`.
    InlineSrcBlock {
        /// Language of the block.
        language: CowStr<'a>,
        /// Optional header arguments, without the surrounding brackets.
        parameters: Option<CowStr<'a>>,
        /// Body of the block.
        value: CowStr<'a>,
    },
    /// An inline babel call `call_name[inside](args)[end]`.
    InlineBabelCall {
        /// Name of the code block to call.
        call: CowStr<'a>,
        /// Header arguments inside the first optional bracket.
        inside_header: Option<CowStr<'a>>,
        /// Arguments passed to the block.
        arguments: Option<CowStr<'a>>,
        /// Header arguments inside the trailing optional bracket.
        end_header: Option<CowStr<'a>>,
    },
    /// A macro `{{{key(arg1,arg2)}}}`.
    Macro {
        /// Macro name, lower-cased.
        key: CowStr<'a>,
        /// Arguments. Commas may be escaped with a backslash.
        args: Vec<CowStr<'a>>,
    },
    /// A target `<<name>>`.
    Target(CowStr<'a>),
    /// A statistics cookie `[33%]` or `[1/3]`. The value includes the brackets.
    StatisticsCookie(CowStr<'a>),
    /// A timestamp.
    Timestamp(Timestamp<'a>),
    /// A footnote reference `[fn:label]` without an inline definition.
    ///
    /// References carrying an inline definition (`[fn:label:text]` and
    /// `[fn::text]`) are containers, see [`Tag::FootnoteReference`].
    FootnoteReference(CowStr<'a>),
    /// A citation reference inside a [`Tag::Citation`]: `@key`, optionally
    /// with a prefix and a suffix.
    CitationReference {
        /// The citation key, without the leading `@`.
        key: CowStr<'a>,
        /// Raw text before the key, if any.
        prefix: Option<CowStr<'a>>,
        /// Raw text after the key, if any.
        suffix: Option<CowStr<'a>>,
    },
    /// A line break `\\` at the end of a line inside a paragraph.
    LineBreak,
    /// A newline inside a paragraph or any other object container.
    SoftBreak,
    /// A horizontal rule (a line of five or more dashes).
    Rule,
    /// A keyword line `#+KEY: value`.
    ///
    /// Affiliated keywords (`#+NAME:`, `#+CAPTION:`, `#+ATTR_HTML:`, ...)
    /// attached to the next element are emitted, in source order, right before
    /// the `Start` event of that element with `affiliated` set to `true`.
    Keyword(Keyword<'a>),
    /// A comment: one or more lines starting with `#`. The value is the
    /// comment text with the leading `# ` removed from each line.
    Comment(CowStr<'a>),
    /// A fixed-width area: one or more lines starting with `: `. The value
    /// is the text with the leading `: ` removed from each line.
    FixedWidth(CowStr<'a>),
    /// A LaTeX environment `\begin{env} ... \end{env}`. The value is the
    /// full raw environment.
    LatexEnvironment(CowStr<'a>),
    /// A babel call line `#+CALL: name[inside](args)[end]`.
    BabelCall {
        /// Name of the code block to call.
        call: CowStr<'a>,
        /// Header arguments inside the first optional bracket.
        inside_header: Option<CowStr<'a>>,
        /// Arguments passed to the block.
        arguments: Option<CowStr<'a>>,
        /// Header arguments inside the trailing optional bracket.
        end_header: Option<CowStr<'a>>,
    },
    /// A diary sexp line `%%(...)`. The value is the full line.
    DiarySexp(CowStr<'a>),
    /// A `CLOCK:` line.
    Clock {
        /// The clock timestamp (inactive, possibly a range). `None` for a
        /// malformed line such as `CLOCK: => 1:00`.
        timestamp: Option<Timestamp<'a>>,
        /// The `=> HH:MM` duration, if the clock is closed.
        duration: Option<CowStr<'a>>,
        /// Whether the clock is still running.
        status: ClockStatus,
    },
    /// The planning line (`SCHEDULED:`, `DEADLINE:`, `CLOSED:`) right below a
    /// headline.
    Planning {
        /// `CLOSED:` timestamp.
        closed: Option<Timestamp<'a>>,
        /// `SCHEDULED:` timestamp.
        scheduled: Option<Timestamp<'a>>,
        /// `DEADLINE:` timestamp.
        deadline: Option<Timestamp<'a>>,
    },
    /// A `:KEY: value` line inside a property drawer.
    NodeProperty {
        /// The property name, without colons.
        key: CowStr<'a>,
        /// The property value (may be empty).
        value: CowStr<'a>,
    },
}

/// Tags for container events.
#[derive(Clone, Debug, PartialEq)]
pub enum Tag<'a> {
    // ---- Elements -------------------------------------------------------
    /// A headline. Its content is: a [`Tag::Title`] holding the title
    /// objects, an optional [`Tag::Section`] holding the content before the
    /// first sub-headline, then the sub-headlines.
    Headline(Headline<'a>),
    /// The title of a headline or inline task (objects).
    Title,
    /// An inline task (requires an `org-inlinetask`-style headline of
    /// `inlinetask_min_level` stars or more). Its content is a
    /// [`Tag::Title`] followed by the task body (elements).
    Inlinetask(Headline<'a>),
    /// The content of a headline before its first sub-headline, or the content
    /// of the document before the first headline.
    Section,
    /// A paragraph (objects).
    Paragraph,
    /// A plain list (items).
    PlainList {
        /// Ordered, unordered or descriptive.
        kind: ListKind,
    },
    /// A list item. Its content is an optional [`Tag::ItemTag`] (for
    /// descriptive lists) followed by elements.
    ///
    /// The range of the `Start` event begins at the bullet, like
    /// `org-element`'s `:begin`. The contents (`:contents-begin`) start
    /// after the bullet, counter, checkbox and tag, at the range of the
    /// first event following `Start(Item)` or `End(ItemTag)`; an item with
    /// no element has no contents.
    Item(Item<'a>),
    /// The tag (term) of an item in a descriptive list (objects). The range
    /// of the `Start` and `End` events is the tag as written, before the
    /// ` ::`.
    ItemTag,
    /// A table. Its content is table rows.
    Table {
        /// Whether this is an Org table or a `table.el` table.
        kind: TableKind,
        /// The `#+TBLFM:` lines following the table.
        formulas: Vec<CowStr<'a>>,
    },
    /// A table row. Standard rows contain [`Tag::TableCell`]s, rule rows
    /// (`|---+---|`) are empty.
    TableRow {
        /// Standard or rule.
        kind: TableRowKind,
    },
    /// A table cell (objects).
    ///
    /// Its range runs from after the previous `|` to after its own closing
    /// `|` (or the end of the line), like `org-element`'s `:begin` and
    /// `:end`; the trimmed cell text is the content between.
    TableCell,
    /// `#+BEGIN_CENTER` block (elements).
    CenterBlock,
    /// `#+BEGIN_QUOTE` block (elements).
    QuoteBlock,
    /// `#+BEGIN_VERSE` block (objects; newlines are significant).
    VerseBlock,
    /// `#+BEGIN_name` block with an unknown name (elements).
    SpecialBlock {
        /// The block name, as written.
        name: CowStr<'a>,
        /// Text following the name on the `#+BEGIN` line, raw (as
        /// `org-element`'s `:parameters`). Use
        /// [`Parser::new_inline`](crate::Parser::new_inline) to read it as
        /// objects.
        parameters: Option<CowStr<'a>>,
    },
    /// `#+BEGIN_SRC` block. Its content is a single [`Event::Text`] with
    /// the body as written; the HTML renderer removes the common
    /// indentation unless the `-i` switch is given, as `ox-html` does.
    SrcBlock {
        /// The language, if given.
        language: Option<CowStr<'a>>,
        /// Switches such as `-n`, `+n 10`, `-r`, `-i`, `-l "fmt"`.
        switches: Option<CowStr<'a>>,
        /// Header arguments such as `:results output`.
        parameters: Option<CowStr<'a>>,
    },
    /// `#+BEGIN_EXAMPLE` block. Its content is a single [`Event::Text`].
    ExampleBlock {
        /// Switches such as `-n`.
        switches: Option<CowStr<'a>>,
    },
    /// `#+BEGIN_EXPORT backend` block. Its content is a single
    /// [`Event::Text`].
    ExportBlock {
        /// The backend name.
        backend: CowStr<'a>,
    },
    /// `#+BEGIN_COMMENT` block. Its content is a single [`Event::Text`].
    CommentBlock,
    /// A drawer `:NAME: ... :END:` (elements).
    Drawer {
        /// The drawer name, as written.
        name: CowStr<'a>,
    },
    /// A property drawer `:PROPERTIES: ... :END:`. Its content is
    /// [`Event::NodeProperty`] events.
    PropertyDrawer,
    /// A dynamic block `#+BEGIN: name params ... #+END:` (elements).
    DynamicBlock {
        /// The block name.
        name: CowStr<'a>,
        /// The parameters following the name.
        parameters: Option<CowStr<'a>>,
    },
    /// A footnote definition `[fn:label] ...` (elements).
    FootnoteDefinition {
        /// The footnote label.
        label: CowStr<'a>,
    },

    // ---- Objects --------------------------------------------------------
    /// `*bold*`.
    Bold,
    /// `/italic/`.
    Italic,
    /// `_underline_`.
    Underline,
    /// `+strike-through+`.
    StrikeThrough,
    /// A link. Its content is the description objects, if any.
    Link(Link<'a>),
    /// A subscript `a_b` or `a_{b}`.
    Subscript {
        /// Whether the subscript is written with braces.
        use_brackets: bool,
    },
    /// A superscript `a^b` or `a^{b}`.
    Superscript {
        /// Whether the superscript is written with braces.
        use_brackets: bool,
    },
    /// A radio target `<<<target>>>` (objects).
    RadioTarget,
    /// A footnote reference with an inline definition, `[fn:label:def]` or
    /// `[fn::def]`. Its content is the definition objects.
    FootnoteReference {
        /// The label, if any.
        label: Option<CowStr<'a>>,
    },
    /// A citation `[cite/style:common prefix; @key; common suffix]`. Its
    /// content is [`Event::CitationReference`] events.
    Citation {
        /// The citation style, if any (text after `cite/`).
        style: Option<CowStr<'a>>,
        /// Raw common prefix.
        prefix: Option<CowStr<'a>>,
        /// Raw common suffix.
        suffix: Option<CowStr<'a>>,
    },
}

impl<'a> Tag<'a> {
    /// The [`TagEnd`] that closes this tag.
    pub fn to_end(&self) -> TagEnd {
        match self {
            Tag::Headline(_) => TagEnd::Headline,
            Tag::Title => TagEnd::Title,
            Tag::Inlinetask(_) => TagEnd::Inlinetask,
            Tag::Section => TagEnd::Section,
            Tag::Paragraph => TagEnd::Paragraph,
            Tag::PlainList { kind } => TagEnd::PlainList(*kind),
            Tag::Item(_) => TagEnd::Item,
            Tag::ItemTag => TagEnd::ItemTag,
            Tag::Table { .. } => TagEnd::Table,
            Tag::TableRow { .. } => TagEnd::TableRow,
            Tag::TableCell => TagEnd::TableCell,
            Tag::CenterBlock => TagEnd::CenterBlock,
            Tag::QuoteBlock => TagEnd::QuoteBlock,
            Tag::VerseBlock => TagEnd::VerseBlock,
            Tag::SpecialBlock { .. } => TagEnd::SpecialBlock,
            Tag::SrcBlock { .. } => TagEnd::SrcBlock,
            Tag::ExampleBlock { .. } => TagEnd::ExampleBlock,
            Tag::ExportBlock { .. } => TagEnd::ExportBlock,
            Tag::CommentBlock => TagEnd::CommentBlock,
            Tag::Drawer { .. } => TagEnd::Drawer,
            Tag::PropertyDrawer => TagEnd::PropertyDrawer,
            Tag::DynamicBlock { .. } => TagEnd::DynamicBlock,
            Tag::FootnoteDefinition { .. } => TagEnd::FootnoteDefinition,
            Tag::Bold => TagEnd::Bold,
            Tag::Italic => TagEnd::Italic,
            Tag::Underline => TagEnd::Underline,
            Tag::StrikeThrough => TagEnd::StrikeThrough,
            Tag::Link(_) => TagEnd::Link,
            Tag::Subscript { .. } => TagEnd::Subscript,
            Tag::Superscript { .. } => TagEnd::Superscript,
            Tag::RadioTarget => TagEnd::RadioTarget,
            Tag::FootnoteReference { .. } => TagEnd::FootnoteReference,
            Tag::Citation { .. } => TagEnd::Citation,
        }
    }
}

/// The end of a container. Carries only the information needed to know which
/// container is being closed. Variants mirror [`Tag`].
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagEnd {
    Headline,
    Title,
    Inlinetask,
    Section,
    Paragraph,
    PlainList(ListKind),
    Item,
    ItemTag,
    Table,
    TableRow,
    TableCell,
    CenterBlock,
    QuoteBlock,
    VerseBlock,
    SpecialBlock,
    SrcBlock,
    ExampleBlock,
    ExportBlock,
    CommentBlock,
    Drawer,
    PropertyDrawer,
    DynamicBlock,
    FootnoteDefinition,
    Bold,
    Italic,
    Underline,
    StrikeThrough,
    Link,
    Subscript,
    Superscript,
    RadioTarget,
    FootnoteReference,
    Citation,
}

impl<'a> From<Tag<'a>> for TagEnd {
    fn from(tag: Tag<'a>) -> Self {
        tag.to_end()
    }
}

/// Data attached to a [`Tag::Headline`] or [`Tag::Inlinetask`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Headline<'a> {
    /// Number of stars.
    pub level: usize,
    /// The TODO keyword, if any (e.g. `TODO`, `DONE`).
    pub todo_keyword: Option<CowStr<'a>>,
    /// Whether the TODO keyword is of type "todo" or "done".
    pub todo_type: Option<TodoType>,
    /// The priority cookie: `[#A]` or a numeric `[#7]` / `[#10]`.
    pub priority: Option<Priority>,
    /// The tags, without colons.
    pub tags: Vec<CowStr<'a>>,
    /// Whether the headline is commented out (`COMMENT` keyword).
    pub commented: bool,
    /// Whether the headline carries the `ARCHIVE` tag.
    pub archived: bool,
    /// Whether this is the footnote section (`* Footnotes` by default).
    pub footnote_section: bool,
    /// The raw title text, before object parsing. Does not include the TODO
    /// keyword, priority, `COMMENT` keyword or tags.
    pub raw_title: CowStr<'a>,
}

/// A headline priority cookie.
///
/// Emacs stores a letter priority as its character code and a numeric one as
/// the number itself (`org-priority-to-value`), which is what
/// [`Priority::value`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Priority {
    /// `[#A]`, `[#b]`: a single letter, as written.
    Letter(char),
    /// `[#7]`, `[#10]`: a number from 0 to 64.
    Numeric(u8),
}

impl Priority {
    /// The value Emacs' `org-element` reports for this priority: the
    /// character code of a letter, or the number itself.
    pub fn value(self) -> u32 {
        match self {
            Priority::Letter(c) => c as u32,
            Priority::Numeric(n) => u32::from(n),
        }
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Priority::Letter(c) => write!(f, "{c}"),
            Priority::Numeric(n) => write!(f, "{n}"),
        }
    }
}

/// Whether a TODO keyword denotes an open or a closed task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TodoType {
    /// An open task.
    Todo,
    /// A closed task.
    Done,
}

/// Data attached to a [`Tag::Item`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item<'a> {
    /// The bullet, with trailing whitespace: `"- "`, `"1. "`, `"1) "`, `"* "`.
    pub bullet: CowStr<'a>,
    /// The counter set with `[@n]`, if any (the text inside, e.g. `"3"` or
    /// `"c"`).
    pub counter: Option<CowStr<'a>>,
    /// The checkbox, if any.
    pub checkbox: Option<Checkbox>,
}

/// The state of a checkbox in a list item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Checkbox {
    /// `[ ]`
    Off,
    /// `[X]`
    On,
    /// `[-]`
    Trans,
}

/// The kind of a plain list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// Items start with `-`, `+` or `*`.
    Unordered,
    /// Items start with a number or letter followed by `.` or `)`.
    Ordered,
    /// Unordered list whose items have a `term ::` tag.
    Descriptive,
}

/// The kind of a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableKind {
    /// A native Org table (`|` delimited).
    Org,
    /// A `table.el` table (`+-` borders). Its content is a single
    /// [`Event::Text`] with the raw table.
    TableEl,
}

/// The kind of a table row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableRowKind {
    /// A row containing cells.
    Standard,
    /// A horizontal rule `|---+---|`.
    Rule,
}

/// Whether a clock is running or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClockStatus {
    /// The clock has a closing timestamp and a duration.
    Closed,
    /// The clock is still running.
    Running,
}

/// A `#+KEY: value` keyword.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyword<'a> {
    /// The key, upper-cased (`TITLE`, `NAME`, `ATTR_HTML`, ...).
    pub key: CowStr<'a>,
    /// The value, trimmed.
    pub value: CowStr<'a>,
    /// For dual keywords (`#+CAPTION[short]: long`, `#+RESULTS[hash]:`),
    /// the optional value inside brackets.
    pub optional: Option<CowStr<'a>>,
    /// Whether this keyword is affiliated to the element that follows it.
    pub affiliated: bool,
}

/// Data attached to a [`Tag::Link`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link<'a> {
    /// The link type: `https`, `file`, `id`, `fuzzy`, `custom-id`, `coderef`,
    /// `radio`, ... Follows `org-element`'s `:type`. Types that are not
    /// built into Emacs are only recognised when registered with
    /// [`Options::link_types`](crate::Options::link_types); otherwise
    /// `[[media:key]]` is a `fuzzy` link with path `media:key`.
    pub link_type: CowStr<'a>,
    /// The path, i.e. the link without its type prefix.
    pub path: CowStr<'a>,
    /// The full link (type and path) as Emacs' `:raw-link` reports it: for
    /// bracket links, after removing escapes and expanding link
    /// abbreviations; for plain and angle links, the source text.
    pub raw_link: CowStr<'a>,
    /// How the link is written.
    pub format: LinkFormat,
    /// For `file` links, the search option after `::`.
    pub search_option: Option<CowStr<'a>>,
    /// For `file+app` links, the application.
    pub application: Option<CowStr<'a>>,
}

/// The syntax used to write a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkFormat {
    /// `https://example.com`
    Plain,
    /// `<https://example.com>`
    Angle,
    /// `[[https://example.com][description]]`
    Bracket,
}

/// An entity such as `\alpha`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity<'a> {
    /// The entity name (e.g. `alpha`).
    pub name: &'static str,
    /// LaTeX replacement.
    pub latex: &'static str,
    /// Whether the LaTeX replacement must be in math mode.
    pub latex_math: bool,
    /// HTML replacement.
    pub html: &'static str,
    /// ASCII replacement.
    pub ascii: &'static str,
    /// Latin-1 replacement.
    pub latin1: &'static str,
    /// UTF-8 replacement.
    pub utf8: &'static str,
    /// Whether the entity was written with trailing braces (`\alpha{}`).
    pub use_brackets: bool,
    /// Phantom borrow so that user-defined entities can be added later without
    /// breaking the API.
    #[doc(hidden)]
    pub _marker: std::marker::PhantomData<&'a ()>,
}

/// A timestamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timestamp<'a> {
    /// Active, inactive, range or diary.
    pub kind: TimestampKind,
    /// The raw timestamp text, including delimiters.
    pub raw: CowStr<'a>,
    /// The start (or only) date and time.
    pub start: Option<DateTime>,
    /// The end date and time for ranges.
    pub end: Option<DateTime>,
    /// The repeater (`+1w`, `++1m`, `.+1d`), if any.
    pub repeater: Option<Repeater>,
    /// The warning period (`-2d`), if any.
    pub warning: Option<Warning>,
}

/// The kind of a timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimestampKind {
    /// `<2024-01-01 Mon>`
    Active,
    /// `[2024-01-01 Mon]`
    Inactive,
    /// `<2024-01-01 Mon>--<2024-01-02 Tue>` or `<2024-01-01 Mon 10:00-11:00>`
    ActiveRange,
    /// `[2024-01-01 Mon]--[2024-01-02 Tue]`
    InactiveRange,
    /// `<%%(diary-float t 4 2)>`
    Diary,
}

/// A date with an optional time.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: Option<u8>,
    pub minute: Option<u8>,
}

/// A timestamp repeater.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Repeater {
    pub kind: RepeaterKind,
    pub value: u32,
    pub unit: TimeUnit,
}

/// The kind of a repeater.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RepeaterKind {
    /// `+1w`
    Cumulate,
    /// `++1w`
    CatchUp,
    /// `.+1w`
    Restart,
}

/// A timestamp warning period.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Warning {
    pub kind: WarningKind,
    pub value: u32,
    pub unit: TimeUnit,
}

/// The kind of a warning period.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WarningKind {
    /// `-2d`
    All,
    /// `--2d`
    First,
}

/// A unit of time in repeaters and warnings.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    Hour,
    Day,
    Week,
    Month,
    Year,
}
