//! Markdown with mermaid diagrams, rendered for a terminal in one [`Theme`].

pub mod theme;

use merman::render::{
    HeadlessRenderer,
    raster::{RasterError, RasterFitBox, RasterOptions},
};
use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
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

pub enum Chunk<'a> {
    Text(&'a str),
    Mermaid { raw: &'a str, source: String },
    /// A top-level code block or GitHub alert, drawn in a rounded box by [`boxed`].
    Boxed { md: String, title: String, alert: Option<AlertKind> },
}

/// Split markdown into plain text, ```mermaid fenced blocks and boxed blocks, keeping
/// source order.
pub fn split(md: &str) -> Vec<Chunk<'_>> {
    let mut chunks = Vec::new();
    let mut last = 0;
    let mut current: Option<(std::ops::Range<usize>, String)> = None;
    let (mut depth, mut boxed_until) = (0usize, 0);
    for (event, range) in Parser::new_ext(md, Options::ENABLE_GFM).into_offset_iter() {
        // Everything inside a boxed block was already taken with it.
        if range.start < boxed_until {
            continue;
        }
        let boxed = match &event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang)))
                if lang.split_whitespace().next() == Some("mermaid") =>
            {
                current = Some((range.clone(), String::new()));
                None
            }
            Event::Start(Tag::CodeBlock(kind)) if depth == 0 => {
                let title = match kind {
                    CodeBlockKind::Fenced(lang) => lang.split_whitespace().next().unwrap_or_default().to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                Some(Chunk::Boxed { md: md[range.clone()].to_string(), title, alert: None })
            }
            Event::Start(Tag::BlockQuote(Some(kind))) if depth == 0 => {
                let alert = alert_kind(*kind);
                // The alert's content, without its `> ` prefixes and `[!KIND]` line.
                let inner = md[range.clone()]
                    .lines()
                    .skip(1)
                    .map(|l| {
                        let l = l.trim_start();
                        let l = l.strip_prefix('>').unwrap_or(l);
                        l.strip_prefix(' ').unwrap_or(l)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(Chunk::Boxed { md: inner, title: alert.label().to_string(), alert: Some(alert) })
            }
            Event::Text(text) if current.is_some() => {
                current.as_mut().unwrap().1.push_str(text);
                None
            }
            Event::End(TagEnd::CodeBlock) if current.is_some() => {
                let (range, source) = current.take().unwrap();
                chunks.push(Chunk::Text(&md[last..range.start]));
                chunks.push(Chunk::Mermaid { raw: &md[range.clone()], source });
                last = range.end;
                None
            }
            _ => None,
        };
        if let Some(chunk) = boxed {
            chunks.push(Chunk::Text(&md[last..range.start]));
            chunks.push(chunk);
            (last, boxed_until) = (range.end, range.end);
            continue;
        }
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
    }
    chunks.push(Chunk::Text(&md[last..]));
    chunks
}

fn alert_kind(kind: BlockQuoteKind) -> AlertKind {
    match kind {
        BlockQuoteKind::Note => AlertKind::Note,
        BlockQuoteKind::Tip => AlertKind::Tip,
        BlockQuoteKind::Important => AlertKind::Important,
        BlockQuoteKind::Warning => AlertKind::Warning,
        BlockQuoteKind::Caution => AlertKind::Caution,
    }
}

/// `md` rendered at `width - 4` and framed as text lines in a rounded box, titled
/// `title` and colored by `alert` (muted when `None`). Plain lines rather than a
/// bordered widget, so the box scrolls, searches and wraps like any other text.
pub fn boxed(md: &str, title: &str, alert: Option<AlertKind>, theme: &Theme, width: u16) -> Text<'static> {
    let text = markdown(md, theme);
    if width < 8 {
        return text;
    }
    let border = alert.map_or(Style::new().fg(theme::color(theme.muted)), |k| theme.alert(k));
    let max_inner = width - 4;
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    // ponytail: one buffer for the whole box; fine for code blocks, not for 100k-line ones.
    // At least one row, so an empty code block reads as an empty box, as on GitHub.
    let height = u16::try_from(paragraph.line_count(max_inner)).unwrap_or(u16::MAX).max(1);
    let area = Rect::new(0, 0, max_inner, height);
    // Unwritten cells keep an empty symbol, so the used width can be measured.
    let mut blank = Cell::default();
    blank.set_symbol("");
    let mut buffer = Buffer::filled(area, blank);
    paragraph.render(area, &mut buffer);

    // Fit the box to its widest row (and its title), capped by the terminal.
    let label = if title.is_empty() { String::new() } else { format!(" {title} ") };
    let label_width = Span::raw(&label).width() as u16;
    let used = (0..height)
        .flat_map(|y| (0..max_inner).map(move |x| (x, y)))
        .filter(|&p| !buffer[p].symbol().is_empty())
        .map(|(x, y)| x + (Span::raw(buffer[(x, y)].symbol()).width() as u16).max(1))
        .max()
        .unwrap_or(0);
    let inner = used.max(label_width.saturating_sub(1)).min(max_inner);
    let rule = |n: u16| "─".repeat(usize::from(n));

    let mut lines = vec![Line::default(), Line::from(vec![
        Span::styled("╭─", border),
        Span::styled(label, border.add_modifier(Modifier::BOLD)),
        Span::styled(format!("{}╮", rule((inner + 1).saturating_sub(label_width))), border),
    ])];
    for y in 0..height {
        let mut spans = vec![Span::styled("│ ", border)];
        let mut x = 0;
        while x < inner {
            let cell = &buffer[(x, y)];
            let symbol = if cell.symbol().is_empty() { " " } else { cell.symbol() };
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
pub fn markdown(md: &str, theme: &Theme) -> Text<'static> {
    let options = tui_markdown::Options::new(theme.clone()).code_theme(theme.code());
    let text = tui_markdown::from_str_with_options(md, &options);
    let lines = text.lines.into_iter().map(|l| {
        let mut spans: Vec<Span<'static>> =
            l.spans.into_iter().map(|s| Span::styled(s.content.into_owned(), theme.prose(l.style.patch(s.style)))).collect();
        task_checkbox(&mut spans, theme);
        Line::from(spans).style(l.style).alignment(l.alignment.unwrap_or_default())
    });
    Text::from(lines.collect::<Vec<_>>()).style(theme::color(theme.foreground))
}

/// tui-markdown writes task items as `- [x] ` (or `[x] ` after an ordered marker).
/// Draw a checkbox instead, as GitHub does: muted when open, `success` when done.
/// Brackets and ✓ are in nearly every monospace font, so both states render in the
/// terminal font at the same size; ☐ and ☑ often come from different fallback fonts.
fn task_checkbox(spans: &mut [Span<'static>], theme: &Theme) {
    let checkbox = |checked: bool| {
        let (glyph, role) = if checked { ("[✓] ", theme.success) } else { ("[ ] ", theme.muted) };
        (glyph, Style::new().fg(theme::color(role)))
    };
    let first = spans.first().map(|s| s.content.to_string()).unwrap_or_default();
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
pub fn diagram(source: &str, theme: &Theme, max_width_px: u32, cell_h: u32) -> Result<Option<Vec<u8>>, RasterError> {
    let raster = RasterOptions::default().with_fit_to(RasterFitBox::width(max_width_px));
    HeadlessRenderer::new().with_host_theme(&theme.mermaid(cell_h)).render_png_sync(source, &raster)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_mermaid_blocks_in_order() {
        let md = "# Hi\n\n```mermaid\ngraph TD\nA-->B\n```\n\n```rust\nfn x() {}\n```\n\nbye\n";
        let chunks = split(md);
        assert_eq!(chunks.len(), 5);
        assert!(matches!(chunks[0], Chunk::Text(t) if t == "# Hi\n\n"));
        assert!(matches!(&chunks[1], Chunk::Mermaid { source, .. } if source == "graph TD\nA-->B\n"));
        assert!(matches!(&chunks[3], Chunk::Boxed { title, alert: None, .. } if title == "rust"));
        assert!(matches!(chunks[4], Chunk::Text(t) if t.ends_with("bye\n")));
    }

    #[test]
    fn markdown_follows_dracula() {
        use ratatui::style::{Color, Modifier};
        let t = Theme::dracula();
        let text = markdown("# Title **strong**\n\n**bold** *slanted* `code` [link](u)\n\n```rust\nfn main() {}\n```\n", &t);
        let span = |needle: &str| {
            text.lines.iter().flat_map(|l| &l.spans).find(|s| s.content.contains(needle)).unwrap().style
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

    fn boxes(md: &str) -> Vec<(String, String, Option<AlertKind>)> {
        split(md)
            .into_iter()
            .filter_map(|c| match c {
                Chunk::Boxed { md, title, alert } => Some((md, title, alert)),
                _ => None,
            })
            .collect()
    }

    fn rows(text: &Text) -> Vec<String> {
        text.lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
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
            assert_eq!(found, vec![("Body text.".to_string(), kind.label().to_string(), Some(kind))], "{marker}");
        }
    }

    #[test]
    fn alert_keeps_paragraphs_lazy_lines_and_inner_code() {
        let md = "> [!NOTE]\n> First.\nlazy continuation\n>\n> ```sh\n> ls\n> ```\n\nAfter.\n";
        let chunks = split(md);
        let found = boxes(md);
        assert_eq!(found.len(), 1, "the inner code block is not boxed separately");
        assert_eq!(found[0].0, "First.\nlazy continuation\n\n```sh\nls\n```");
        assert!(matches!(chunks.last(), Some(Chunk::Text(t)) if t.trim() == "After."));
    }

    #[test]
    fn only_top_level_code_and_alerts_are_boxed() {
        let md = "> A plain quote.\n\n- item\n\n  ```rust\n  let x = 1;\n  ```\n\nA paragraph.\n\n    indented code\n";
        let found = boxes(md);
        assert_eq!(found.len(), 1, "plain quotes and code in lists stay in the text");
        assert_eq!((found[0].1.as_str(), found[0].2), ("", None), "indented code has no title");
        assert!(found[0].0.contains("indented code"));
        assert!(matches!(&split(md)[0], Chunk::Text(t) if t.contains("```rust")));
    }

    #[test]
    fn box_rows_are_equal_width_and_fit_the_widest_line() {
        let t = Theme::dracula();
        let text = boxed("```rust\nfn main() {}\nlet longer_line = 1;\n```\n", "rust", None, &t, 60);
        let rows = rows(&text);
        assert_eq!(rows.first().map(String::as_str), Some(""), "blank line above");
        assert_eq!(rows.last().map(String::as_str), Some(""), "blank line below");
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
        let narrow = boxed("x", "Important", Some(AlertKind::Important), &t, 60);
        let frame = &narrow.lines[1..narrow.lines.len() - 1];
        assert!(frame.iter().all(|l| l.width() == frame[0].width()));
        assert_eq!(frame[0].width(), " Important ".len() + 3, "title fits exactly");

        let long = boxed(&"word ".repeat(40), "Tip", Some(AlertKind::Tip), &t, 40);
        let frame = &long.lines[1..long.lines.len() - 1];
        assert!(frame.len() > 3, "long text wraps onto several rows");
        assert!(frame.iter().all(|l| l.width() <= 40));
    }

    #[test]
    fn box_aligns_wide_characters() {
        let t = Theme::dracula();
        let text = boxed("猫猫 cat\nascii only line", "", None, &t, 60);
        let frame = &text.lines[1..text.lines.len() - 1];
        assert!(frame.iter().all(|l| l.width() == frame[0].width()), "{:#?}", rows(&text));
    }

    #[test]
    fn box_edge_cases() {
        let t = Theme::dracula();
        // Too narrow for a frame: plain text.
        let plain = boxed("hello", "Note", Some(AlertKind::Note), &t, 7);
        assert!(!rows(&plain).iter().any(|r| r.contains('╭')));
        // An empty code block still gets a well-formed frame.
        let empty = boxed("```\n```\n", "", None, &t, 60);
        let frame = &empty.lines[1..empty.lines.len() - 1];
        assert!(frame.iter().all(|l| l.width() == frame[0].width()), "{:#?}", rows(&empty));
    }

    #[test]
    fn task_items_render_as_checkboxes() {
        use ratatui::style::Color;
        let t = Theme::dracula();
        let text = markdown("- [ ] open\n- [x] done\n  - [ ] nested\n\n1. [x] ordered\n\n- plain\n", &t);
        let rows = rows(&text);
        let rgb = |c: [u8; 3]| Some(Color::Rgb(c[0], c[1], c[2]));
        let row = |needle: &str| rows.iter().position(|r| r.contains(needle)).unwrap();
        assert_eq!(rows[row("open")], "[ ] open");
        assert_eq!(rows[row("done")], "[✓] done");
        assert!(rows[row("nested")].ends_with("[ ] nested") && rows[row("nested")].starts_with(' '), "{rows:#?}");
        assert!(rows[row("ordered")].contains("[✓] ordered"), "{rows:#?}");
        assert!(rows[row("plain")].contains("- plain"), "ordinary bullets are untouched");
        assert_eq!(text.lines[row("open")].spans[0].style.fg, rgb(t.muted));
        assert_eq!(text.lines[row("done")].spans[0].style.fg, rgb(t.success));
    }

    #[test]
    fn box_keeps_syntax_highlighting_and_colors_the_border() {
        use ratatui::style::Color;
        let t = Theme::dracula();
        let text = boxed("```rust\nfn main() {}\n```\n", "rust", Some(AlertKind::Warning), &t, 60);
        let span = |needle: &str| text.lines.iter().flat_map(|l| &l.spans).find(|s| s.content == needle).unwrap().style;
        let rgb = |c: [u8; 3]| Some(Color::Rgb(c[0], c[1], c[2]));
        assert_eq!(span("f").fg, rgb(t.keyword), "first cell of `fn` keeps keyword color");
        assert_eq!(span("│ ").fg, rgb(t.warning));
    }
}
