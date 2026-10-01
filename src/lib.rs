//! Documents with mermaid diagrams, rendered for a terminal in one [`Theme`].

pub mod document;
pub mod markdown;
pub mod theme;

use document::{AdmonitionKind, Block, Document, Inline, Node};
use merman::render::{
    HeadlessRenderer,
    raster::{RasterError, RasterFitBox, RasterOptions},
};
use ratatui::{
    buffer::{Buffer, Cell},
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Widget, Wrap},
};
pub use theme::Theme;
pub use tui_markdown::AlertKind;
use tui_markdown::StyleSheet;

/// A stretch of a document the viewer lays out one way.
pub enum Chunk {
    /// Flowing text: paragraphs, headings, lists, tables, nested code.
    Prose(Vec<Block>),
    /// A top-level code block or alert, drawn in a rounded box by [`boxed`].
    Boxed {
        body: Vec<Block>,
        title: String,
        alert: Option<AlertKind>,
    },
    /// A top-level mermaid code block: `lang` is its info string, for showing the
    /// source when the diagram can't be drawn.
    Mermaid { lang: String, source: String },
}

/// Split a document into prose, boxes and diagrams, in order. Only top-level code
/// blocks and alerts are boxed: inside a list or quote they stay part of the text.
/// Diagrams are drawn wherever they are, cutting the list or quote around them.
pub fn chunks(doc: Document) -> Vec<Chunk> {
    let (mut chunks, mut prose) = (Vec::new(), Vec::new());
    for piece in doc.into_iter().flat_map(hoist) {
        let chunk = match piece {
            Piece::Diagram { lang, source } => Chunk::Mermaid { lang, source },
            Piece::Block(Block::Code { lang, code }) => Chunk::Boxed {
                title: lang
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                body: vec![Block::Code { lang, code }],
                alert: None,
            },
            Piece::Block(Block::Admonition { kind, body }) => {
                let alert = alert_kind(kind);
                Chunk::Boxed {
                    body,
                    title: alert.label().to_string(),
                    alert: Some(alert),
                }
            }
            Piece::Block(block) => {
                prose.push(block);
                continue;
            }
        };
        if !prose.is_empty() {
            chunks.push(Chunk::Prose(std::mem::take(&mut prose)));
        }
        chunks.push(chunk);
    }
    if !prose.is_empty() {
        chunks.push(Chunk::Prose(prose));
    }
    chunks
}

/// A block, or a mermaid diagram cut out of one.
enum Piece {
    Block(Block),
    Diagram { lang: String, source: String },
}

impl Piece {
    fn into_block(self) -> Option<Block> {
        match self {
            Piece::Block(b) => Some(b),
            Piece::Diagram { .. } => None,
        }
    }
}

fn is_mermaid(lang: &str) -> bool {
    lang.split_whitespace().next() == Some("mermaid")
}

/// `block` cut around the mermaid diagrams in it and in its lists and quotes, at any
/// depth. A quote goes on after a diagram; the rest of a list item can't stay in the
/// item without a marker, so it follows as plain blocks, and the list resumes at its
/// next item, numbered on. Alerts keep their diagrams: they're drawn in the box.
fn hoist(block: Block) -> Vec<Piece> {
    match block {
        Block::Code { lang, code } if is_mermaid(&lang) => {
            vec![Piece::Diagram { lang, source: code }]
        }
        Block::Quote(body) => {
            let mut pieces = Vec::new();
            let mut quoted = Vec::new();
            for piece in body.into_iter().flat_map(hoist) {
                match piece {
                    Piece::Block(b) => quoted.push(b),
                    diagram => {
                        if !quoted.is_empty() {
                            pieces.push(Piece::Block(Block::Quote(std::mem::take(&mut quoted))));
                        }
                        pieces.push(diagram);
                    }
                }
            }
            if !quoted.is_empty() {
                pieces.push(Piece::Block(Block::Quote(quoted)));
            }
            pieces
        }
        Block::List(list) => {
            let mut pieces = Vec::new();
            let mut items = Vec::new();
            let mut start = list.start;
            for (i, item) in list.items.into_iter().enumerate() {
                let mut body: Vec<Piece> = item.body.into_iter().flat_map(hoist).collect();
                let cut = body.iter().position(|p| matches!(p, Piece::Diagram { .. }));
                let rest = cut.map(|cut| body.split_off(cut));
                items.push(document::Item {
                    task: item.task,
                    body: body.into_iter().filter_map(Piece::into_block).collect(),
                });
                let Some(rest) = rest else { continue };
                pieces.push(Piece::Block(Block::List(document::List {
                    start,
                    tight: list.tight,
                    items: std::mem::take(&mut items),
                })));
                pieces.extend(rest);
                start = list.start.map(|n| n + i as u64 + 1);
            }
            if !items.is_empty() {
                pieces.push(Piece::Block(Block::List(document::List {
                    start,
                    tight: list.tight,
                    items,
                })));
            }
            pieces
        }
        block => vec![Piece::Block(block)],
    }
}

fn alert_kind(kind: AdmonitionKind) -> AlertKind {
    match kind {
        AdmonitionKind::Note => AlertKind::Note,
        AdmonitionKind::Tip => AlertKind::Tip,
        AdmonitionKind::Important => AlertKind::Important,
        AdmonitionKind::Warning => AlertKind::Warning,
        AdmonitionKind::Caution => AlertKind::Caution,
    }
}

/// Blocks as styled text in the theme's colors, through tui-markdown. It only shows a
/// footnote reference as one when the definition is in the same text, and a chunk's
/// definitions are often in a later chunk: a stand-in definition is added for each,
/// and its lines dropped again.
pub fn render(blocks: &[Block], theme: &Theme) -> Text<'static> {
    let (mut refs, mut defs) = (Vec::<String>::new(), Vec::new());
    document::visit(blocks, &mut |node| match node {
        Node::Inline(Inline::FootnoteRef(label)) => refs.push(label.to_lowercase()),
        Node::Block(Block::Footnote { label, .. }) => defs.push(label.to_lowercase()),
        _ => {}
    });
    let mut missing: Vec<String> = Vec::new();
    for label in refs {
        if !defs.contains(&label) && !missing.contains(&label) {
            missing.push(label);
        }
    }
    let mut md = markdown::write(blocks);
    for label in &missing {
        md.push_str(&format!("\n[^{label}]: -\n"));
    }
    let mut text = markdown(&md, theme);
    // tui-markdown starts a definition with a blank line, then `[label]: `.
    if let Some(first) = missing.first() {
        let start = format!("[{first}]: ");
        if let Some(i) = text
            .lines
            .iter()
            .position(|l| l.spans.first().is_some_and(|s| s.content == start))
        {
            text.lines.truncate(i);
            if text.lines.last().is_some_and(|l| l.width() == 0) {
                text.lines.pop();
            }
        }
    }
    text
}

/// `body` rendered at `width - 4` and framed as text lines in a rounded box, titled
/// `title` and colored by `alert` (muted when `None`). Plain lines rather than a
/// bordered widget, so the box scrolls, searches and wraps like any other text.
pub fn boxed(
    body: &[Block],
    title: &str,
    alert: Option<AlertKind>,
    theme: &Theme,
    width: u16,
) -> Text<'static> {
    let text = render(body, theme);
    if width < 8 {
        return text;
    }
    let border = alert.map_or(Style::new().fg(theme::color(theme.muted)), |k| {
        theme.alert(k)
    });
    let max_inner = width - 4;
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    // ponytail: one buffer for the whole box; fine for code blocks, not for 100k-line ones.
    // At least one row, so an empty code block reads as an empty box, as on GitHub.
    let height = u16::try_from(paragraph.line_count(max_inner))
        .unwrap_or(u16::MAX)
        .max(1);
    let area = Rect::new(0, 0, max_inner, height);
    // Unwritten cells keep an empty symbol, so the used width can be measured.
    let mut blank = Cell::default();
    blank.set_symbol("");
    let mut buffer = Buffer::filled(area, blank);
    paragraph.render(area, &mut buffer);

    // Fit the box to its widest row (and its title), capped by the terminal.
    let label = if title.is_empty() {
        String::new()
    } else {
        format!(" {title} ")
    };
    let label_width = Span::raw(&label).width() as u16;
    let used = (0..height)
        .flat_map(|y| (0..max_inner).map(move |x| (x, y)))
        .filter(|&p| !buffer[p].symbol().is_empty())
        .map(|(x, y)| x + (Span::raw(buffer[(x, y)].symbol()).width() as u16).max(1))
        .max()
        .unwrap_or(0);
    let inner = used.max(label_width.saturating_sub(1)).min(max_inner);
    let rule = |n: u16| "─".repeat(usize::from(n));

    let mut lines = vec![
        Line::default(),
        Line::from(vec![
            Span::styled("╭─", border),
            Span::styled(label, border.add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{}╮", rule((inner + 1).saturating_sub(label_width))),
                border,
            ),
        ]),
    ];
    for y in 0..height {
        let mut spans = vec![Span::styled("│ ", border)];
        let mut x = 0;
        while x < inner {
            let cell = &buffer[(x, y)];
            let symbol = if cell.symbol().is_empty() {
                " "
            } else {
                cell.symbol()
            };
            // Skip the cells a wide character covers, as the renderer left them.
            x += (Span::raw(symbol).width() as u16).max(1);
            spans.push(Span::styled(symbol.to_string(), cell.style()));
        }
        spans.push(Span::styled(" │", border));
        lines.push(Line::from(spans));
    }
    lines.push(Line::styled(format!("╰{}╯", rule(inner + 2)), border));
    lines.push(Line::default());
    Text::from(lines)
}

/// Styled markdown text (no diagrams) in the theme's colors.
fn markdown(md: &str, theme: &Theme) -> Text<'static> {
    let options = tui_markdown::Options::new(theme.clone()).code_theme(theme.code());
    let text = tui_markdown::from_str_with_options(md, &options);
    let lines = text.lines.into_iter().map(|l| {
        let mut spans: Vec<Span<'static>> = l
            .spans
            .into_iter()
            .map(|s| Span::styled(s.content.into_owned(), theme.prose(l.style.patch(s.style))))
            .collect();
        task_checkbox(&mut spans, theme);
        Line::from(spans)
            .style(l.style)
            .alignment(l.alignment.unwrap_or_default())
    });
    Text::from(lines.collect::<Vec<_>>()).style(theme::color(theme.foreground))
}

/// Whether `line` is a heading. tui-markdown gives heading lines the heading style as
/// their line style; nothing else gets it (table headers style their spans instead).
pub fn is_heading(line: &Line, theme: &Theme) -> bool {
    (1..=6).any(|level| line.style == theme.heading(level))
}

/// tui-markdown writes task items as `- [x] ` (or `[x] ` after an ordered marker).
/// Draw a checkbox instead, as GitHub does: muted when open, `success` when done.
/// Brackets and ✓ are in nearly every monospace font, so both states render in the
/// terminal font at the same size; ☐ and ☑ often come from different fallback fonts.
fn task_checkbox(spans: &mut [Span<'static>], theme: &Theme) {
    let checkbox = |checked: bool| {
        let (glyph, role) = if checked {
            ("[✓] ", theme.success)
        } else {
            ("[ ] ", theme.muted)
        };
        (glyph, Style::new().fg(theme::color(role)))
    };
    let first = spans
        .first()
        .map(|s| s.content.to_string())
        .unwrap_or_default();
    for (marker, checked) in [("- [ ] ", false), ("- [x] ", true), ("- [X] ", true)] {
        if let Some(indent) = first.strip_suffix(marker) {
            let (glyph, style) = checkbox(checked);
            spans[0] = Span::styled(format!("{indent}{glyph}"), style);
            return;
        }
    }
    if let Some(second) = spans.get_mut(1) {
        let checked = match second.content.as_ref() {
            "[ ] " => false,
            "[x] " | "[X] " => true,
            _ => return,
        };
        let (glyph, style) = checkbox(checked);
        *second = Span::styled(glyph, style);
    }
}

/// A mermaid diagram as a PNG with a transparent background, at most `max_width_px`
/// wide, with text sized for `cell_h`-pixel terminal rows. `None` if it isn't mermaid.
pub fn diagram(
    source: &str,
    theme: &Theme,
    max_width_px: u32,
    cell_h: u32,
) -> Result<Option<Vec<u8>>, RasterError> {
    let raster = RasterOptions::default().with_fit_to(RasterFitBox::width(max_width_px));
    HeadlessRenderer::new()
        .with_host_theme(&theme.mermaid(cell_h))
        .render_png_sync(source, &raster)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(md: &str) -> Vec<Chunk> {
        chunks(markdown::parse(md))
    }

    /// `boxed`, for a box of markdown.
    fn boxed_md(
        md: &str,
        title: &str,
        alert: Option<AlertKind>,
        t: &Theme,
        w: u16,
    ) -> Text<'static> {
        boxed(&markdown::parse(md), title, alert, t, w)
    }

    #[test]
    fn splits_mermaid_blocks_in_order() {
        let md = "# Hi\n\n```mermaid\ngraph TD\nA-->B\n```\n\n```rust\nfn x() {}\n```\n\nbye\n";
        let chunks = split(md);
        assert_eq!(chunks.len(), 4);
        assert!(matches!(&chunks[0], Chunk::Prose(b) if *b == markdown::parse("# Hi")));
        assert!(
            matches!(&chunks[1], Chunk::Mermaid { lang, source } if lang == "mermaid" && source == "graph TD\nA-->B\n")
        );
        assert!(matches!(&chunks[2], Chunk::Boxed { title, alert: None, .. } if title == "rust"));
        assert!(matches!(&chunks[3], Chunk::Prose(b) if *b == markdown::parse("bye")));
    }

    /// Each chunk as it shows: prose as its rendered rows (blank ones dropped), a
    /// diagram as `mermaid: ` and its source.
    fn outline(md: &str) -> Vec<String> {
        let t = Theme::dracula();
        split(md)
            .into_iter()
            .map(|c| match c {
                Chunk::Prose(blocks) => rows(&render(&blocks, &t))
                    .into_iter()
                    .filter(|r| !r.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
                Chunk::Mermaid { source, .. } => format!("mermaid: {}", source.trim_end()),
                Chunk::Boxed { title, .. } => format!("box: {title}"),
            })
            .collect()
    }

    #[test]
    fn nested_diagrams_are_drawn_like_top_level_ones() {
        let md = "- before\n\n  ```mermaid\n  graph TD\n  ```\n\n  after\n- next\n\n3. three\n4. four\n\n   ```mermaid\n   pie\n   ```\n5. five\n\n> quoted\n>\n> ```mermaid\n> graph LR\n> ```\n>\n> still quoted\n";
        assert_eq!(
            outline(md),
            [
                "- before",
                "mermaid: graph TD",
                // The rest of the item can't stay in it without a marker: it follows as text.
                "after\n- next\n3. three\n4. four",
                "mermaid: pie",
                "5. five\n> quoted",
                "mermaid: graph LR",
                "> still quoted",
            ]
        );
        let deep = "- outer\n  - inner\n\n    ```mermaid\n    deep\n    ```\n\n    tail\n- last\n";
        assert_eq!(
            outline(deep),
            ["- outer\n    - inner", "mermaid: deep", "tail\n- last"],
            "both lists are cut"
        );
        let alert = "> [!NOTE]\n> ```mermaid\n> graph\n> ```\n";
        assert_eq!(
            outline(alert),
            ["box: Note"],
            "an alert keeps its diagram in the box"
        );
    }

    #[test]
    fn footnotes_and_reference_links_reach_across_boxes() {
        let t = Theme::dracula();
        let md = "See [the docs][d] and a note[^1].\n\n```sh\nls\n```\n\n[d]: https://example.com\n[^1]: The footnote.\n";
        let chunks = split(md);
        let (Chunk::Prose(first), Chunk::Prose(last)) = (&chunks[0], &chunks[2]) else {
            panic!("prose, box, prose")
        };
        let text = render(first, &t);
        assert_eq!(
            rows(&text),
            ["See the docs (https://example.com) and a note[1]."],
            "no stand-in left"
        );
        let note = text.lines[0]
            .spans
            .iter()
            .find(|s| s.content == "[1]")
            .unwrap();
        assert_eq!(
            note.style.fg,
            Some(theme::color(t.info)),
            "styled as a footnote reference"
        );
        assert_eq!(rows(&render(last, &t)), ["[1]: The footnote."]);
    }

    #[test]
    fn markdown_follows_dracula() {
        use ratatui::style::{Color, Modifier};
        let t = Theme::dracula();
        let text = markdown(
            "# Title **strong**\n\n**bold** *slanted* `code` [link](u)\n\n```rust\nfn main() {}\n```\n",
            &t,
        );
        let span = |needle: &str| {
            text.lines
                .iter()
                .flat_map(|l| &l.spans)
                .find(|s| s.content.contains(needle))
                .unwrap()
                .style
        };
        let rgb = |c: [u8; 3]| Some(Color::Rgb(c[0], c[1], c[2]));
        assert_eq!(span("Title").fg, rgb(t.heading));
        assert!(span("Title").add_modifier.contains(Modifier::BOLD));
        assert_eq!(span("strong").fg, rgb(t.heading)); // bold in a heading stays heading-colored
        assert_eq!(span("bold").fg, rgb(t.bold));
        assert_eq!(span("slanted").fg, rgb(t.italic));
        assert_eq!(span("code").fg, rgb(t.code));
        assert_eq!(span("link").fg, rgb(t.link));
        assert_eq!(span("fn").fg, rgb(t.keyword)); // keyword
        assert_eq!(span("main").fg, rgb(t.function)); // function name
    }

    fn boxes(md: &str) -> Vec<(Document, String, Option<AlertKind>)> {
        split(md)
            .into_iter()
            .filter_map(|c| match c {
                Chunk::Boxed { body, title, alert } => Some((body, title, alert)),
                _ => None,
            })
            .collect()
    }

    fn rows(text: &Text) -> Vec<String> {
        text.lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn splits_every_alert_kind_without_its_marker() {
        let kinds = [
            ("NOTE", AlertKind::Note),
            ("TIP", AlertKind::Tip),
            ("IMPORTANT", AlertKind::Important),
            ("WARNING", AlertKind::Warning),
            ("CAUTION", AlertKind::Caution),
        ];
        for (marker, kind) in kinds {
            let found = boxes(&format!("> [!{marker}]\n> Body text.\n"));
            assert_eq!(
                found,
                vec![(
                    markdown::parse("Body text."),
                    kind.label().to_string(),
                    Some(kind)
                )],
                "{marker}"
            );
        }
    }

    #[test]
    fn alert_keeps_paragraphs_lazy_lines_and_inner_code() {
        let md = "> [!NOTE]\n> First.\nlazy continuation\n>\n> ```sh\n> ls\n> ```\n\nAfter.\n";
        let chunks = split(md);
        let found = boxes(md);
        assert_eq!(
            found.len(),
            1,
            "the inner code block is not boxed separately"
        );
        assert_eq!(
            found[0].0,
            markdown::parse("First.\nlazy continuation\n\n```sh\nls\n```")
        );
        assert!(matches!(chunks.last(), Some(Chunk::Prose(b)) if *b == markdown::parse("After.")));
    }

    #[test]
    fn only_top_level_code_and_alerts_are_boxed() {
        let md = "> A plain quote.\n\n- item\n\n  ```rust\n  let x = 1;\n  ```\n\nA paragraph.\n\n    indented code\n";
        let found = boxes(md);
        assert_eq!(
            found.len(),
            1,
            "plain quotes and code in lists stay in the text"
        );
        assert_eq!(
            (found[0].1.as_str(), found[0].2),
            ("", None),
            "indented code has no title"
        );
        assert_eq!(found[0].0, markdown::parse("    indented code\n"));
        let Chunk::Prose(text) = &split(md)[0] else {
            panic!("prose first")
        };
        assert!(
            matches!(
                text[..],
                [Block::Quote(_), Block::List(_), Block::Paragraph(_)]
            ),
            "the quote, the list with its code block, and the paragraph: {text:?}"
        );
    }

    #[test]
    fn box_rows_are_equal_width_and_fit_the_widest_line() {
        let t = Theme::dracula();
        let text = boxed_md(
            "```rust\nfn main() {}\nlet longer_line = 1;\n```\n",
            "rust",
            None,
            &t,
            60,
        );
        let rows = rows(&text);
        assert_eq!(
            rows.first().map(String::as_str),
            Some(""),
            "blank line above"
        );
        assert_eq!(
            rows.last().map(String::as_str),
            Some(""),
            "blank line below"
        );
        let frame = &text.lines[1..text.lines.len() - 1];
        let width = "let longer_line = 1;".len() + 4;
        assert!(frame.iter().all(|l| l.width() == width), "{rows:#?}");
        assert!(rows[1].starts_with("╭─ rust ─") && rows[1].ends_with('╮'));
        assert!(rows[2].starts_with("│ fn main() {}") && rows[2].ends_with(" │"));
        assert!(rows[rows.len() - 2].starts_with('╰') && rows[rows.len() - 2].ends_with('╯'));
    }

    #[test]
    fn box_widens_for_its_title_and_wraps_at_the_terminal() {
        let t = Theme::dracula();
        let narrow = boxed_md("x", "Important", Some(AlertKind::Important), &t, 60);
        let frame = &narrow.lines[1..narrow.lines.len() - 1];
        assert!(frame.iter().all(|l| l.width() == frame[0].width()));
        assert_eq!(
            frame[0].width(),
            " Important ".len() + 3,
            "title fits exactly"
        );

        let long = boxed_md(&"word ".repeat(40), "Tip", Some(AlertKind::Tip), &t, 40);
        let frame = &long.lines[1..long.lines.len() - 1];
        assert!(frame.len() > 3, "long text wraps onto several rows");
        assert!(frame.iter().all(|l| l.width() <= 40));
    }

    #[test]
    fn box_aligns_wide_characters() {
        let t = Theme::dracula();
        let text = boxed_md("猫猫 cat\nascii only line", "", None, &t, 60);
        let frame = &text.lines[1..text.lines.len() - 1];
        assert!(
            frame.iter().all(|l| l.width() == frame[0].width()),
            "{:#?}",
            rows(&text)
        );
    }

    #[test]
    fn box_edge_cases() {
        let t = Theme::dracula();
        // Too narrow for a frame: plain text.
        let plain = boxed_md("hello", "Note", Some(AlertKind::Note), &t, 7);
        assert!(!rows(&plain).iter().any(|r| r.contains('╭')));
        // An empty code block still gets a well-formed frame.
        let empty = boxed_md("```\n```\n", "", None, &t, 60);
        let frame = &empty.lines[1..empty.lines.len() - 1];
        assert!(
            frame.iter().all(|l| l.width() == frame[0].width()),
            "{:#?}",
            rows(&empty)
        );
    }

    #[test]
    fn task_items_render_as_checkboxes() {
        use ratatui::style::Color;
        let t = Theme::dracula();
        let text = markdown(
            "- [ ] open\n- [x] done\n  - [ ] nested\n\n1. [x] ordered\n\n- plain\n",
            &t,
        );
        let rows = rows(&text);
        let rgb = |c: [u8; 3]| Some(Color::Rgb(c[0], c[1], c[2]));
        let row = |needle: &str| rows.iter().position(|r| r.contains(needle)).unwrap();
        assert_eq!(rows[row("open")], "[ ] open");
        assert_eq!(rows[row("done")], "[✓] done");
        assert!(
            rows[row("nested")].ends_with("[ ] nested") && rows[row("nested")].starts_with(' '),
            "{rows:#?}"
        );
        assert!(rows[row("ordered")].contains("[✓] ordered"), "{rows:#?}");
        assert!(
            rows[row("plain")].contains("- plain"),
            "ordinary bullets are untouched"
        );
        assert_eq!(text.lines[row("open")].spans[0].style.fg, rgb(t.muted));
        assert_eq!(text.lines[row("done")].spans[0].style.fg, rgb(t.success));
    }

    #[test]
    fn box_keeps_syntax_highlighting_and_colors_the_border() {
        use ratatui::style::Color;
        let t = Theme::dracula();
        let text = boxed_md(
            "```rust\nfn main() {}\n```\n",
            "rust",
            Some(AlertKind::Warning),
            &t,
            60,
        );
        let span = |needle: &str| {
            text.lines
                .iter()
                .flat_map(|l| &l.spans)
                .find(|s| s.content == needle)
                .unwrap()
                .style
        };
        let rgb = |c: [u8; 3]| Some(Color::Rgb(c[0], c[1], c[2]));
        assert_eq!(
            span("f").fg,
            rgb(t.keyword),
            "first cell of `fn` keeps keyword color"
        );
        assert_eq!(span("│ ").fg, rgb(t.warning));
    }

    #[test]
    fn detects_headings_by_line_style_only() {
        let t = Theme::dracula();
        let md = "# One\n\n## Two\n\n###### Six\n\n**bold** text\n\n> quote\n\n| Head |\n| ---- |\n| cell |\n\n```sh\n# comment\n```\n";
        let text = markdown(md, &t);
        let headings: Vec<String> = text
            .lines
            .iter()
            .filter(|l| is_heading(l, &t))
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(headings, ["One", "Two", "Six"]);
    }
}
