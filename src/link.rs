//! Clickable links: link text shown without its URL, opened by the terminal through
//! OSC 8 hyperlinks. [`hide_urls`] tags the text when it's rendered, the tag rides
//! through wrapping and scrolling in the style, and [`apply`] turns tagged cells into
//! hyperlinks once they're on screen.

use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::Rect,
    style::Color,
    text::{Span, Text},
};
use std::num::NonZeroU16;

/// Link `id` (from 1) as an underline color: never drawn, [`apply`] clears it.
fn tag(id: usize) -> Color {
    Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8)
}

fn id(color: Color) -> Option<usize> {
    match color {
        Color::Rgb(r, g, b) => {
            Some(usize::from(r) << 16 | usize::from(g) << 8 | usize::from(b)).filter(|&id| id > 0)
        }
        _ => None,
    }
}

/// Drop the ` (url)` tui-markdown writes after each link of `pending` (the links in
/// `text` as `(url, text)`, in order), and tag the link's text with its URL's place
/// in `urls`.
pub fn hide_urls(text: &mut Text<'static>, pending: &[(String, String)], urls: &mut Vec<String>) {
    let mut pending = pending.iter().peekable();
    for line in &mut text.lines {
        let old = std::mem::take(&mut line.spans);
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(old.len());
        let mut i = 0;
        while i < old.len() {
            let link = pending.peek().filter(|(url, _)| {
                old[i].content == " ("
                    && old.get(i + 1).is_some_and(|s| s.content == url.as_str())
                    && old.get(i + 2).is_some_and(|s| s.content == ")")
            });
            let Some((url, label)) = link else {
                spans.push(old[i].clone());
                i += 1;
                continue;
            };
            // A URL is document content: no control characters, so it can't end the
            // escape sequence early and slip in one of its own.
            urls.push(url.chars().filter(|c| !c.is_control()).collect());
            let tag = tag(urls.len());
            // The link's text: the spans just before, as many as its text fills. Its
            // style can't tell: code in a link isn't underlined, a heading is.
            let mut left = label.chars().count();
            for span in spans.iter_mut().rev() {
                let n = span.content.chars().count();
                if n > left {
                    break;
                }
                left -= n;
                span.style.underline_color = Some(tag);
                if left == 0 {
                    break;
                }
            }
            pending.next();
            i += 3;
        }
        line.spans = spans;
    }
}

/// Make the tagged cells in `area` hyperlinks to their URL in `urls`, and untag them.
/// Each cell is a whole link on its own, so a partial redraw can't leave one open.
pub fn apply(buf: &mut Buffer, area: Rect, urls: &[String]) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            let Some(url) = id(cell.underline_color).and_then(|id| urls.get(id - 1)) else {
                continue;
            };
            let symbol = cell.symbol().to_string();
            let width = (Span::raw(symbol.as_str()).width() as u16).max(1);
            cell.set_symbol(&format!("\x1b]8;;{url}\x1b\\{symbol}\x1b]8;;\x1b\\"))
                .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(width).unwrap()));
            cell.underline_color = Color::Reset;
        }
    }
}

/// A cell's text, without the hyperlink [`apply`] may have wrapped it in.
pub fn visible(symbol: &str) -> &str {
    match symbol
        .strip_prefix("\x1b]8;;")
        .and_then(|s| s.split_once("\x1b\\"))
    {
        Some((_, rest)) => rest.strip_suffix("\x1b]8;;\x1b\\").unwrap_or(rest),
        None => symbol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Theme, markdown, render};

    /// `md` rendered with clickable links: its rows, each link's text, and the URLs.
    fn linked(md: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut links = Some(Vec::new());
        let text = render(&markdown::parse(md), &Theme::dracula(), &mut links);
        let urls = links.unwrap();
        let rows = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let labels = (1..=urls.len())
            .map(|id| {
                let spans = text.lines.iter().flat_map(|l| &l.spans);
                spans
                    .filter(|s| s.style.underline_color == Some(tag(id)))
                    .map(|s| s.content.as_ref())
                    .collect()
            })
            .collect();
        (rows, labels, urls)
    }

    #[test]
    fn links_show_their_text_tagged_with_their_url() {
        let (rows, labels, urls) = linked(
            "# Heading [link](https://h)\n\nSee [the **docs**](https://d), [`code` too](https://c), [a](https://a)[b](https://b) and <https://auto>.\n",
        );
        assert_eq!(rows[0], "Heading link");
        assert_eq!(rows[2], "See the docs, code too, ab and https://auto.");
        assert_eq!(
            labels,
            ["link", "the docs", "code too", "a", "b", "https://auto"],
            "exactly each link's text, whatever its style"
        );
        assert_eq!(
            urls,
            [
                "https://h",
                "https://d",
                "https://c",
                "https://a",
                "https://b",
                "https://auto"
            ]
        );
    }

    #[test]
    fn without_a_link_table_urls_stay_in_the_text() {
        let text = render(
            &markdown::parse("[docs](https://d)\n"),
            &Theme::dracula(),
            &mut None,
        );
        let row: String = text.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(row, "docs (https://d)");
    }

    #[test]
    fn urls_lose_control_characters() {
        let mut text = Text::from(ratatui::text::Line::from(vec![
            Span::raw("x"),
            Span::raw(" ("),
            Span::raw("https://e\x1b]8;;evil\x07"),
            Span::raw(")"),
        ]));
        let mut urls = Vec::new();
        let pending = [("https://e\x1b]8;;evil\x07".to_string(), "x".to_string())];
        hide_urls(&mut text, &pending, &mut urls);
        assert_eq!(urls, ["https://e]8;;evil"]);
    }

    #[test]
    fn tagged_cells_become_whole_hyperlinks_and_read_back_as_text() {
        let mut buf = Buffer::with_lines(["a猫 b"]);
        for x in 0..3 {
            buf[(x, 0)].underline_color = tag(1);
        }
        let area = buf.area;
        apply(&mut buf, area, &["https://x".to_string()]);
        assert_eq!(
            buf[(0, 0)].symbol(),
            "\x1b]8;;https://x\x1b\\a\x1b]8;;\x1b\\"
        );
        assert_eq!(
            buf[(1, 0)].diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap())
        );
        assert_eq!(
            buf[(0, 0)].underline_color,
            Color::Reset,
            "the tag is never drawn"
        );
        assert_eq!(buf[(4, 0)].symbol(), "b", "untagged cells stay as they are");
        assert_eq!(visible(buf[(1, 0)].symbol()), "猫");
        assert_eq!(visible("b"), "b");
    }
}
