//! A document as yamdview shows it, whatever it was written in: what it contains,
//! not how it was spelled. Parsers build one ([`crate::markdown::parse`]), the
//! viewer lays it out.

pub type Document = Vec<Block>;

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading {
        level: u8,
        content: Vec<Inline>,
        attrs: HeadingAttrs,
    },
    Paragraph(Vec<Inline>),
    Code {
        /// The language and anything written after it; empty when there's none.
        lang: String,
        code: String,
    },
    List(List),
    Quote(Vec<Block>),
    Admonition {
        kind: AdmonitionKind,
        body: Vec<Block>,
    },
    Table(Table),
    Definitions(Vec<Definition>),
    Footnote {
        label: String,
        body: Vec<Block>,
    },
    /// Front matter, such as YAML between `---` lines, shown as written.
    Metadata(String),
    Html(String),
    Rule,
}

/// An id, classes and key-value attributes given to a heading, shown after it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeadingAttrs {
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct List {
    /// The first number of a numbered list; `None` for bullets.
    pub start: Option<u64>,
    /// No blank lines between items.
    pub tight: bool,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// A task item's checkbox: `Some(true)` when done.
    pub task: Option<bool>,
    pub body: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AdmonitionKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub align: Vec<Align>,
    pub head: Vec<Vec<Inline>>,
    pub rows: Vec<Vec<Vec<Inline>>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub term: Vec<Inline>,
    /// One body per definition the term has.
    pub details: Vec<Vec<Block>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Superscript(Vec<Inline>),
    Subscript(Vec<Inline>),
    Link {
        url: String,
        title: String,
        content: Vec<Inline>,
    },
    Image {
        url: String,
        title: String,
        alt: Vec<Inline>,
    },
    FootnoteRef(String),
    Math {
        display: bool,
        tex: String,
    },
    Html(String),
    SoftBreak,
    HardBreak,
}

/// A block or an inline, as [`visit`] hands them out.
pub enum Node<'a> {
    Block(&'a Block),
    Inline(&'a Inline),
}

/// Calls `f` on every block and inline in `blocks`, nested ones included, each
/// before what it contains.
pub fn visit<'a>(blocks: &'a [Block], f: &mut impl FnMut(Node<'a>)) {
    for block in blocks {
        f(Node::Block(block));
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => visit_inlines(content, f),
            Block::List(list) => list.items.iter().for_each(|item| visit(&item.body, f)),
            Block::Quote(body) | Block::Admonition { body, .. } | Block::Footnote { body, .. } => {
                visit(body, f)
            }
            Block::Table(table) => table
                .head
                .iter()
                .chain(table.rows.iter().flatten())
                .for_each(|cell| visit_inlines(cell, f)),
            Block::Definitions(definitions) => {
                for d in definitions {
                    visit_inlines(&d.term, f);
                    d.details.iter().for_each(|body| visit(body, f));
                }
            }
            Block::Code { .. } | Block::Metadata(_) | Block::Html(_) | Block::Rule => {}
        }
    }
}

fn visit_inlines<'a>(content: &'a [Inline], f: &mut impl FnMut(Node<'a>)) {
    for inline in content {
        f(Node::Inline(inline));
        match inline {
            Inline::Emphasis(c)
            | Inline::Strong(c)
            | Inline::Strikethrough(c)
            | Inline::Superscript(c)
            | Inline::Subscript(c)
            | Inline::Link { content: c, .. }
            | Inline::Image { alt: c, .. } => visit_inlines(c, f),
            _ => {}
        }
    }
}
