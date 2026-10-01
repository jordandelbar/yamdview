//! Markdown to [`Document`]: the one place that knows Markdown syntax.

use crate::document::{
    AdmonitionKind, Align, Block, Definition, Document, HeadingAttrs, Inline, Item, List, Table,
};
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag};

/// The extensions tui-markdown parses with, so a document reads the same to both.
pub fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT
        | Options::ENABLE_MATH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_GFM
        | Options::ENABLE_TABLES
}

pub fn parse(md: &str) -> Document {
    blocks(tree(Parser::new_ext(md, options())))
}

/// pulldown-cmark's events, nested: each tag holds what came between its start and end.
enum Node<'a> {
    Tag(Tag<'a>, Vec<Node<'a>>),
    Leaf(Event<'a>),
}

fn tree<'a>(events: impl Iterator<Item = Event<'a>>) -> Vec<Node<'a>> {
    let mut stack = vec![(None, Vec::new())];
    for event in events {
        match event {
            Event::Start(tag) => stack.push((Some(tag), Vec::new())),
            Event::End(_) => {
                // pulldown-cmark balances its events, so a tag is open here.
                let (tag, children) = stack.pop().unwrap();
                let parent = &mut stack.last_mut().unwrap().1;
                parent.push(Node::Tag(tag.unwrap(), children));
            }
            leaf => stack.last_mut().unwrap().1.push(Node::Leaf(leaf)),
        }
    }
    stack.swap_remove(0).1
}

fn is_inline(node: &Node) -> bool {
    match node {
        Node::Tag(tag, _) => matches!(
            tag,
            Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::Link { .. }
                | Tag::Image { .. }
        ),
        Node::Leaf(event) => !matches!(event, Event::Rule | Event::Html(_)),
    }
}

/// Block nodes, with runs of bare inlines (a tight list item's text) as paragraphs.
fn blocks(nodes: Vec<Node>) -> Vec<Block> {
    let (mut out, mut run) = (Vec::new(), Vec::new());
    for node in nodes {
        if is_inline(&node) {
            run.push(node);
            continue;
        }
        if !run.is_empty() {
            out.push(Block::Paragraph(inlines(std::mem::take(&mut run))));
        }
        out.extend(block(node));
    }
    if !run.is_empty() {
        out.push(Block::Paragraph(inlines(run)));
    }
    out
}

fn block(node: Node) -> Option<Block> {
    let (tag, children) = match node {
        Node::Tag(tag, children) => (tag, children),
        Node::Leaf(Event::Rule) => return Some(Block::Rule),
        Node::Leaf(_) => return None,
    };
    Some(match tag {
        Tag::Paragraph => Block::Paragraph(inlines(children)),
        Tag::Heading {
            level,
            id,
            classes,
            attrs,
        } => Block::Heading {
            level: level as u8,
            content: inlines(children),
            attrs: HeadingAttrs {
                id: id.map(|s| s.to_string()),
                classes: classes.iter().map(|s| s.to_string()).collect(),
                attrs: attrs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.as_ref().map(|v| v.to_string())))
                    .collect(),
            },
        },
        Tag::BlockQuote(None) => Block::Quote(blocks(children)),
        Tag::BlockQuote(Some(kind)) => Block::Admonition {
            kind: match kind {
                BlockQuoteKind::Note => AdmonitionKind::Note,
                BlockQuoteKind::Tip => AdmonitionKind::Tip,
                BlockQuoteKind::Important => AdmonitionKind::Important,
                BlockQuoteKind::Warning => AdmonitionKind::Warning,
                BlockQuoteKind::Caution => AdmonitionKind::Caution,
            },
            body: blocks(children),
        },
        Tag::CodeBlock(kind) => Block::Code {
            lang: match kind {
                CodeBlockKind::Fenced(lang) => lang.to_string(),
                CodeBlockKind::Indented => String::new(),
            },
            code: text(&children),
        },
        Tag::HtmlBlock => Block::Html(text(&children)),
        Tag::MetadataBlock(_) => Block::Metadata(text(&children)),
        Tag::List(start) => {
            // Loose lists wrap item text in paragraphs; tight ones leave it bare.
            let tight = !children.iter().any(|item| {
                matches!(item, Node::Tag(_, body)
                    if body.iter().any(|n| matches!(n, Node::Tag(Tag::Paragraph, _))))
            });
            Block::List(List {
                start,
                tight,
                items: children.into_iter().filter_map(item).collect(),
            })
        }
        Tag::FootnoteDefinition(label) => Block::Footnote {
            label: label.to_string(),
            body: blocks(children),
        },
        Tag::DefinitionList => {
            let mut definitions: Vec<Definition> = Vec::new();
            for child in children {
                match child {
                    Node::Tag(Tag::DefinitionListTitle, term) => definitions.push(Definition {
                        term: inlines(term),
                        details: Vec::new(),
                    }),
                    Node::Tag(Tag::DefinitionListDefinition, body) => {
                        if definitions.is_empty() {
                            definitions.push(Definition {
                                term: Vec::new(),
                                details: Vec::new(),
                            });
                        }
                        definitions.last_mut().unwrap().details.push(blocks(body));
                    }
                    _ => {}
                }
            }
            Block::Definitions(definitions)
        }
        Tag::Table(align) => {
            let mut table = Table {
                align: align
                    .iter()
                    .map(|a| match a {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect(),
                head: Vec::new(),
                rows: Vec::new(),
            };
            let cells = |row: Vec<Node>| -> Vec<Vec<Inline>> {
                row.into_iter()
                    .filter_map(|cell| match cell {
                        Node::Tag(Tag::TableCell, content) => Some(inlines(content)),
                        _ => None,
                    })
                    .collect()
            };
            for child in children {
                match child {
                    Node::Tag(Tag::TableHead, row) => table.head = cells(row),
                    Node::Tag(Tag::TableRow, row) => table.rows.push(cells(row)),
                    _ => {}
                }
            }
            Block::Table(table)
        }
        // Inline tags are taken by `blocks`, item and table parts by their parents.
        _ => return None,
    })
}

/// A list item, with its task checkbox lifted out of its text.
fn item(node: Node) -> Option<Item> {
    let Node::Tag(Tag::Item, mut body) = node else {
        return None;
    };
    let is_marker = |n: &Node| matches!(n, Node::Leaf(Event::TaskListMarker(_)));
    // Tight items start with the marker; loose ones start their first paragraph with it.
    let first = match body.first_mut() {
        Some(Node::Tag(Tag::Paragraph, text)) => text,
        _ => &mut body,
    };
    let task = match first.first() {
        Some(Node::Leaf(Event::TaskListMarker(done))) => Some(*done),
        _ => None,
    };
    if first.first().is_some_and(is_marker) {
        first.remove(0);
    }
    Some(Item {
        task,
        body: blocks(body),
    })
}

/// Inline nodes, with neighbouring text merged: pulldown-cmark splits it at escapes.
fn inlines(nodes: Vec<Node>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for node in nodes {
        let inline = match node {
            Node::Leaf(event) => match event {
                Event::Text(s) => Inline::Text(s.to_string()),
                Event::Code(s) => Inline::Code(s.to_string()),
                Event::InlineMath(s) => Inline::Math {
                    display: false,
                    tex: s.to_string(),
                },
                Event::DisplayMath(s) => Inline::Math {
                    display: true,
                    tex: s.to_string(),
                },
                Event::InlineHtml(s) | Event::Html(s) => Inline::Html(s.to_string()),
                Event::FootnoteReference(s) => Inline::FootnoteRef(s.to_string()),
                Event::SoftBreak => Inline::SoftBreak,
                Event::HardBreak => Inline::HardBreak,
                // Task markers belong to their item, rules to the blocks.
                _ => continue,
            },
            Node::Tag(tag, children) => match tag {
                Tag::Emphasis => Inline::Emphasis(inlines(children)),
                Tag::Strong => Inline::Strong(inlines(children)),
                Tag::Strikethrough => Inline::Strikethrough(inlines(children)),
                Tag::Superscript => Inline::Superscript(inlines(children)),
                Tag::Subscript => Inline::Subscript(inlines(children)),
                Tag::Link {
                    dest_url, title, ..
                } => Inline::Link {
                    url: dest_url.to_string(),
                    title: title.to_string(),
                    content: inlines(children),
                },
                Tag::Image {
                    dest_url, title, ..
                } => Inline::Image {
                    url: dest_url.to_string(),
                    title: title.to_string(),
                    alt: inlines(children),
                },
                _ => continue,
            },
        };
        match (out.last_mut(), inline) {
            (Some(Inline::Text(prev)), Inline::Text(next)) => prev.push_str(&next),
            (_, inline) => out.push(inline),
        }
    }
    out
}

/// The text of a code, HTML or metadata block, in one piece.
fn text(nodes: &[Node]) -> String {
    nodes
        .iter()
        .filter_map(|n| match n {
            Node::Leaf(Event::Text(s) | Event::Html(s)) => Some(s.as_ref()),
            _ => None,
        })
        .collect()
}

/// Markdown that parses back to `doc`, for renderers that only take Markdown text.
/// Text is escaped wholesale (a backslash before every ASCII punctuation character is
/// always a valid escape), so nothing in it can turn into syntax.
pub fn write(doc: &[Block]) -> String {
    let mut md = write_blocks(doc, "\n\n");
    md.push('\n');
    md
}

/// Blocks joined by `sep`: a blank line, or a newline inside a tight list item.
fn write_blocks(doc: &[Block], sep: &str) -> String {
    let mut parts = Vec::new();
    // Two lists in a row with the same marker would merge into one: alternate it.
    let mut prev_list: Option<(bool, bool)> = None;
    for b in doc {
        parts.push(match b {
            Block::List(list) => {
                let ordered = list.start.is_some();
                let alt = prev_list.is_some_and(|(o, alt)| o == ordered && !alt);
                prev_list = Some((ordered, alt));
                write_list(list, alt)
            }
            b => {
                prev_list = None;
                write_block(b)
            }
        });
    }
    parts.join(sep)
}

fn write_block(b: &Block) -> String {
    match b {
        Block::Heading {
            level,
            content,
            attrs,
        } => {
            let mut parts: Vec<String> = attrs.id.iter().map(|id| format!("#{id}")).collect();
            parts.extend(attrs.classes.iter().map(|c| format!(".{c}")));
            parts.extend(attrs.attrs.iter().map(|(k, v)| match v {
                Some(v) => format!("{k}={v}"),
                None => k.clone(),
            }));
            let attrs = if parts.is_empty() {
                String::new()
            } else {
                format!(" {{{}}}", parts.join(" "))
            };
            let hashes = "#".repeat(usize::from(*level));
            format!("{hashes} {}{attrs}", write_inlines(content, false))
        }
        Block::Paragraph(content) => write_inlines(content, false),
        Block::Code { lang, code } => {
            // Tildes when the info string has a backtick, which a backtick fence can't take.
            let c = if lang.contains('`') { '~' } else { '`' };
            let fence = c.to_string().repeat(3.max(longest_run(code, c) + 1));
            let nl = if code.is_empty() || code.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            format!("{fence}{lang}\n{code}{nl}{fence}")
        }
        Block::Quote(body) => indent(&write_blocks(body, "\n\n"), "> ", "> "),
        Block::Admonition { kind, body } => {
            let label = match kind {
                AdmonitionKind::Note => "NOTE",
                AdmonitionKind::Tip => "TIP",
                AdmonitionKind::Important => "IMPORTANT",
                AdmonitionKind::Warning => "WARNING",
                AdmonitionKind::Caution => "CAUTION",
            };
            let body = write_blocks(body, "\n\n");
            indent(&format!("[!{label}]\n{body}"), "> ", "> ")
        }
        Block::List(list) => write_list(list, false),
        Block::Table(table) => {
            let row = |cells: &[Vec<Inline>]| {
                let cells: Vec<String> = cells.iter().map(|c| write_inlines(c, true)).collect();
                format!("| {} |", cells.join(" | "))
            };
            let rule: Vec<&str> = table
                .align
                .iter()
                .map(|a| match a {
                    Align::None => "---",
                    Align::Left => ":--",
                    Align::Center => ":-:",
                    Align::Right => "--:",
                })
                .collect();
            let mut lines = vec![row(&table.head), format!("| {} |", rule.join(" | "))];
            lines.extend(table.rows.iter().map(|r| row(r)));
            lines.join("\n")
        }
        Block::Definitions(definitions) => definitions
            .iter()
            .map(|d| {
                let mut lines = vec![write_inlines(&d.term, false)];
                for detail in &d.details {
                    lines.push(indent(&write_blocks(detail, "\n\n"), ": ", "  "));
                }
                lines.join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        Block::Footnote { label, body } => indent(
            &write_blocks(body, "\n\n"),
            &format!("[^{label}]: "),
            "    ",
        ),
        Block::Metadata(yaml) => {
            let nl = if yaml.ends_with('\n') { "" } else { "\n" };
            format!("---\n{yaml}{nl}---")
        }
        Block::Html(html) => html.trim_end_matches('\n').to_string(),
        // Not `---`: right under a paragraph line, that underlines a heading.
        Block::Rule => "***".to_string(),
    }
}

/// `alt` picks the other marker (`*` or `)`), to keep apart lists written one after another.
fn write_list(list: &List, alt: bool) -> String {
    let sep = if list.tight { "\n" } else { "\n\n" };
    list.items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = match list.start {
                None if alt => "* ".to_string(),
                None => "- ".to_string(),
                Some(n) => format!("{}{} ", n + i as u64, if alt { ')' } else { '.' }),
            };
            let task = match item.task {
                Some(true) => "[x] ",
                Some(false) => "[ ] ",
                None => "",
            };
            let body = format!("{task}{}", write_blocks(&item.body, sep));
            indent(&body, &marker, &" ".repeat(marker.len()))
        })
        .collect::<Vec<_>>()
        .join(sep)
}

/// `text` with `first` before its first line and `rest` before the others. Blank lines
/// get the prefix without its trailing space: `>` keeps them in a quote.
fn indent(text: &str, first: &str, rest: &str) -> String {
    if text.is_empty() {
        return first.trim_end().to_string();
    }
    text.lines()
        .enumerate()
        .map(|(i, line)| {
            let prefix = if i == 0 { first } else { rest };
            if line.is_empty() {
                prefix.trim_end().to_string()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Inline content; `table` escapes the pipes a table cell would split code at.
fn write_inlines(content: &[Inline], table: bool) -> String {
    let mut md = String::new();
    for inline in content {
        match inline {
            Inline::Text(s) => md.push_str(&escape(s)),
            Inline::Code(code) => md.push_str(&code_span(code, table)),
            Inline::Emphasis(c) => md.push_str(&format!("*{}*", write_inlines(c, table))),
            Inline::Strong(c) => md.push_str(&format!("**{}**", write_inlines(c, table))),
            Inline::Strikethrough(c) => md.push_str(&format!("~~{}~~", write_inlines(c, table))),
            Inline::Superscript(c) => md.push_str(&format!("^{}^", write_inlines(c, table))),
            Inline::Subscript(c) => md.push_str(&format!("~{}~", write_inlines(c, table))),
            Inline::Link {
                url,
                title,
                content,
            } => md.push_str(&format!(
                "[{}]({})",
                write_inlines(content, table),
                destination(url, title)
            )),
            Inline::Image { url, title, alt } => md.push_str(&format!(
                "![{}]({})",
                write_inlines(alt, table),
                destination(url, title)
            )),
            Inline::FootnoteRef(label) => md.push_str(&format!("[^{label}]")),
            Inline::Math { display, tex } => {
                let d = if *display { "$$" } else { "$" };
                md.push_str(&format!("{d}{tex}{d}"));
            }
            Inline::Html(html) => md.push_str(html),
            Inline::SoftBreak => md.push('\n'),
            Inline::HardBreak => md.push_str("\\\n"),
        }
    }
    md
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A code span that reads back as exactly `code`: fenced by more backticks than it
/// holds, and padded by a space on each side, which the parser strips again.
fn code_span(code: &str, table: bool) -> String {
    let code = if table {
        code.replace('|', "\\|")
    } else {
        code.to_string()
    };
    let ticks = "`".repeat(longest_run(&code, '`') + 1);
    // Only spaces: the parser keeps padding there, so don't add any.
    if code.chars().all(|c| c == ' ') {
        format!("{ticks}{code}{ticks}")
    } else {
        format!("{ticks} {code} {ticks}")
    }
}

/// A link or image destination in angle brackets (spaces and parentheses allowed),
/// with its title when it has one.
fn destination(url: &str, title: &str) -> String {
    let url: String = url
        .chars()
        .flat_map(|c| {
            ['\\']
                .into_iter()
                .filter(move |_| "<>\\".contains(c))
                .chain([c])
        })
        .collect();
    if title.is_empty() {
        format!("<{url}>")
    } else {
        let title: String = title
            .chars()
            .flat_map(|c| {
                ['\\']
                    .into_iter()
                    .filter(move |_| "\"\\".contains(c))
                    .chain([c])
            })
            .collect();
        format!("<{url}> \"{title}\"")
    }
}

/// The longest run of `c` in `s`.
fn longest_run(s: &str, c: char) -> usize {
    s.split(|x| x != c).map(str::len).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Inline {
        Inline::Text(s.into())
    }

    fn para(s: &str) -> Block {
        Block::Paragraph(vec![t(s)])
    }

    #[test]
    fn lists_keep_tightness_tasks_and_nesting() {
        let tight = parse("- [x] done\n- b\n  - nested\n");
        let nested = Block::List(List {
            start: None,
            tight: true,
            items: vec![Item {
                task: None,
                body: vec![para("nested")],
            }],
        });
        assert_eq!(
            tight,
            [Block::List(List {
                start: None,
                tight: true,
                items: vec![
                    Item {
                        task: Some(true),
                        body: vec![para("done")],
                    },
                    Item {
                        task: None,
                        body: vec![para("b"), nested],
                    },
                ],
            })]
        );
        let loose = parse("3. [ ] open\n\n4. b\n");
        let Block::List(list) = &loose[0] else {
            panic!("{loose:?}")
        };
        assert_eq!((list.start, list.tight), (Some(3), false));
        assert_eq!(
            list.items[0].task,
            Some(false),
            "marker inside the paragraph"
        );
        assert_eq!(list.items[0].body, [para("open")]);
    }

    #[test]
    fn blocks_keep_what_the_viewer_shows() {
        let doc = parse(
            "---\ntitle: x\n---\n\n# H {#id .c k=v}\n\n> [!NOTE]\n> n\n\n> q\n\n    indented\n\n```rust ignore\nfn x() {}\n```\n\n<div>\nhi\n</div>\n\n***\n",
        );
        assert_eq!(
            doc,
            [
                Block::Metadata("title: x\n".into()),
                Block::Heading {
                    level: 1,
                    content: vec![t("H")],
                    attrs: HeadingAttrs {
                        id: Some("id".into()),
                        classes: vec!["c".into()],
                        attrs: vec![("k".into(), Some("v".into()))],
                    },
                },
                Block::Admonition {
                    kind: AdmonitionKind::Note,
                    body: vec![para("n")],
                },
                Block::Quote(vec![para("q")]),
                Block::Code {
                    lang: String::new(),
                    code: "indented\n".into(),
                },
                Block::Code {
                    lang: "rust ignore".into(),
                    code: "fn x() {}\n".into(),
                },
                Block::Html("<div>\nhi\n</div>\n".into()),
                Block::Rule,
            ]
        );
    }

    #[test]
    fn tables_definitions_footnotes_and_inlines() {
        let doc = parse(
            "| a | b |\n|:-|-:|\n| 1 | *2* |\n\nterm\n: one\n: two\n\nx\\*y <b>z</b> $m$ ~s~ [l](u \"t\") ![i](p)[^1]\n\n[^1]: foot\n",
        );
        assert_eq!(
            doc[0],
            Block::Table(Table {
                align: vec![Align::Left, Align::Right],
                head: vec![vec![t("a")], vec![t("b")]],
                rows: vec![vec![vec![t("1")], vec![Inline::Emphasis(vec![t("2")])]]],
            })
        );
        assert_eq!(
            doc[1],
            Block::Definitions(vec![Definition {
                term: vec![t("term")],
                details: vec![vec![para("one")], vec![para("two")]],
            }])
        );
        assert_eq!(
            doc[2],
            Block::Paragraph(vec![
                t("x*y "),
                Inline::Html("<b>".into()),
                t("z"),
                Inline::Html("</b>".into()),
                t(" "),
                Inline::Math {
                    display: false,
                    tex: "m".into()
                },
                t(" "),
                Inline::Subscript(vec![t("s")]),
                t(" "),
                Inline::Link {
                    url: "u".into(),
                    title: "t".into(),
                    content: vec![t("l")],
                },
                t(" "),
                Inline::Image {
                    url: "p".into(),
                    title: String::new(),
                    alt: vec![t("i")],
                },
                Inline::FootnoteRef("1".into()),
            ]),
            "escaped text merged into one run"
        );
        assert_eq!(
            doc[3],
            Block::Footnote {
                label: "1".into(),
                body: vec![para("foot")],
            }
        );
    }

    /// All the text in `doc`, in order, as the parser's events carry it.
    fn plain(doc: &[Block], out: &mut String) {
        fn inline(i: &[Inline], out: &mut String) {
            for i in i {
                match i {
                    Inline::Text(s)
                    | Inline::Code(s)
                    | Inline::Html(s)
                    | Inline::FootnoteRef(s)
                    | Inline::Math { tex: s, .. } => out.push_str(s),
                    Inline::Emphasis(c)
                    | Inline::Strong(c)
                    | Inline::Strikethrough(c)
                    | Inline::Superscript(c)
                    | Inline::Subscript(c)
                    | Inline::Link { content: c, .. }
                    | Inline::Image { alt: c, .. } => inline(c, out),
                    Inline::SoftBreak | Inline::HardBreak => {}
                }
            }
        }
        for b in doc {
            match b {
                Block::Heading { content: c, .. } | Block::Paragraph(c) => inline(c, out),
                Block::Code { code: s, .. } | Block::Metadata(s) | Block::Html(s) => {
                    out.push_str(s)
                }
                Block::List(l) => l.items.iter().for_each(|i| plain(&i.body, out)),
                Block::Quote(b)
                | Block::Admonition { body: b, .. }
                | Block::Footnote { body: b, .. } => plain(b, out),
                Block::Table(t) => t
                    .head
                    .iter()
                    .chain(t.rows.iter().flatten())
                    .for_each(|c| inline(c, out)),
                Block::Definitions(d) => d.iter().for_each(|d| {
                    inline(&d.term, out);
                    d.details.iter().for_each(|b| plain(b, out));
                }),
                Block::Rule => {}
            }
        }
    }

    #[test]
    fn real_documents_keep_all_their_text() {
        for md in [
            include_str!("../README.md"),
            include_str!("../docs/usage.md"),
            include_str!("../examples/showcase.md"),
        ] {
            let events: String = Parser::new_ext(md, options())
                .filter_map(|e| match e {
                    Event::Text(s)
                    | Event::Code(s)
                    | Event::Html(s)
                    | Event::InlineHtml(s)
                    | Event::FootnoteReference(s)
                    | Event::InlineMath(s)
                    | Event::DisplayMath(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect();
            let mut doc = String::new();
            plain(&parse(md), &mut doc);
            assert!(!doc.is_empty());
            assert_eq!(doc, events);
        }
    }

    /// Markdown chosen to trip the writer: syntax characters in text, backticks and
    /// pipes in code, lists that would merge, nesting that needs indentation.
    const EDGES: &[&str] = &[
        "All punctuation as text: !\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~ and \\\\ backslash.\n",
        "# Heading with trailing hashes \\#\\# and *em* {#id .class key=value}\n",
        "1986\\. A great year.\n\n\\- not a list\n\n\\> not a quote\n",
        "Code: `` a`b ``, ` `` `, `   `, and `|` pipes.\n\n| a | b |\n| - | :-: |\n| `x\\|y` | \\| *c* |\n",
        "````\n```\nnested fence\n```\n````\n\n~~~ lang`tick\nx\n~~~\n",
        "- a\n- b\n\n* c\n* d\n\n- e\n\n1. one\n2. two\n\n1) three\n",
        "- loose\n\n  second paragraph\n- item\n\n  > quoted\n  >\n  > more\n",
        "- [x] done\n- [ ] open\n  1. nested\n  2. list\n\n     with a paragraph\n",
        "7. starts at seven\n8. next\n",
        "> outer\n>\n> > inner\n>\n> - list in quote\n\n> [!WARNING]\n> body\n>\n> ```sh\n> code in alert\n> ```\n",
        "[link with (parens) and spaces](<https://example.com/a b> \"Title \\\"q\\\"\") and <https://auto.link>\n\n![alt *em*](img.png \"t\")\n",
        "Hard\\\nbreak and soft\nbreak, ~~strike~~, ~sub~, $x^2$ and $$\\sum$$.\n",
        "Text[^n] here.\n\n[^n]: A footnote\n\n    with a second paragraph.\n",
        "Term\n: Definition one\n: Definition two\n\nOther\n: More\n",
        "<details>\n<summary>Hi</summary>\n</details>\n\nInline <kbd>Ctrl</kbd> html.\n",
        "Para\n\n***\n\n- tight item\n  ***\n- next\n",
        "---\ntitle: Front matter\n---\n\n# After\n",
        "```\n```\n\n```rust\nno newline at end\n```\n",
    ];

    fn documents() -> Vec<&'static str> {
        let mut docs = vec![
            include_str!("../README.md"),
            include_str!("../docs/usage.md"),
            include_str!("../examples/showcase.md"),
        ];
        docs.extend(EDGES);
        docs
    }

    #[test]
    fn written_markdown_parses_back_to_the_same_document() {
        for md in documents() {
            let doc = parse(md);
            let written = write(&doc);
            assert_eq!(parse(&written), doc, "from:\n{md}\nwritten:\n{written}");
        }
    }

    /// `text` as it looks: neighbouring spans of one style merged, since escapes split
    /// the text tui-markdown gets into more pieces without changing what's drawn.
    fn drawn(text: ratatui::text::Text<'static>) -> Vec<ratatui::text::Line<'static>> {
        use ratatui::text::{Line, Span};
        text.lines
            .into_iter()
            .map(|line| {
                let mut spans: Vec<Span> = Vec::new();
                for span in line.spans {
                    match spans.last_mut() {
                        Some(last) if last.style == span.style => {
                            last.content = format!("{}{}", last.content, span.content).into();
                        }
                        _ => spans.push(span),
                    }
                }
                Line { spans, ..line }
            })
            .collect()
    }

    #[test]
    fn written_markdown_renders_like_the_original() {
        let theme = crate::Theme::dracula();
        for md in documents() {
            let written = write(&parse(md));
            let (got, want) = (
                drawn(crate::markdown(&written, &theme)),
                drawn(crate::markdown(md, &theme)),
            );
            if let Some(i) = (0..got.len().max(want.len())).find(|&i| got.get(i) != want.get(i)) {
                panic!(
                    "line {i} differs\nwritten: {:?}\noriginal: {:?}\nfrom:\n{md}",
                    got.get(i),
                    want.get(i)
                );
            }
        }
    }
}
