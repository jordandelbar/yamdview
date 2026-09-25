//! Markdown with mermaid diagrams, rendered for a terminal in one [`Theme`].

pub mod theme;

use merman::render::{
    HeadlessRenderer,
    raster::{RasterError, RasterFitBox, RasterOptions},
};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::text::{Line, Span, Text};
pub use theme::Theme;

pub enum Chunk<'a> {
    Text(&'a str),
    Mermaid { raw: &'a str, source: String },
}

/// Split markdown into plain text and ```mermaid fenced blocks, keeping source order.
pub fn split(md: &str) -> Vec<Chunk<'_>> {
    let mut chunks = Vec::new();
    let mut last = 0;
    let mut current: Option<(std::ops::Range<usize>, String)> = None;
    for (event, range) in Parser::new(md).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang)))
                if lang.split_whitespace().next() == Some("mermaid") =>
            {
                current = Some((range, String::new()));
            }
            Event::Text(text) if current.is_some() => current.as_mut().unwrap().1.push_str(&text),
            Event::End(TagEnd::CodeBlock) if current.is_some() => {
                let (range, source) = current.take().unwrap();
                chunks.push(Chunk::Text(&md[last..range.start]));
                chunks.push(Chunk::Mermaid { raw: &md[range.clone()], source });
                last = range.end;
            }
            _ => {}
        }
    }
    chunks.push(Chunk::Text(&md[last..]));
    chunks
}

/// Styled markdown text (no diagrams) in the theme's colors.
pub fn markdown(md: &str, theme: &Theme) -> Text<'static> {
    let options = tui_markdown::Options::new(theme.clone()).code_theme(theme.code());
    let text = tui_markdown::from_str_with_options(md, &options);
    let lines = text.lines.into_iter().map(|l| {
        let spans: Vec<Span<'static>> =
            l.spans.into_iter().map(|s| Span::styled(s.content.into_owned(), theme.prose(l.style.patch(s.style)))).collect();
        Line::from(spans).style(l.style).alignment(l.alignment.unwrap_or_default())
    });
    Text::from(lines.collect::<Vec<_>>()).style(theme::color(theme.foreground))
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
        assert_eq!(chunks.len(), 3);
        assert!(matches!(chunks[0], Chunk::Text(t) if t == "# Hi\n\n"));
        assert!(matches!(&chunks[1], Chunk::Mermaid { source, .. } if source == "graph TD\nA-->B\n"));
        assert!(matches!(chunks[2], Chunk::Text(t) if t.contains("```rust") && t.ends_with("bye\n")));
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
}
