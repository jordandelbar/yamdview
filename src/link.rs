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
use std::{
    num::NonZeroU16,
    ops::Range,
    path::{Path, PathBuf},
};

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

/// The URL of a cell [`apply`] made a hyperlink.
pub fn url(symbol: &str) -> Option<&str> {
    let (url, _) = symbol.strip_prefix("\x1b]8;;")?.split_once("\x1b\\")?;
    Some(url)
}

/// A link on screen: its row, the columns its text covers, and its URL.
#[derive(Debug, PartialEq)]
pub struct Spot {
    pub y: u16,
    pub x: Range<u16>,
    pub url: String,
}

/// The links in `buf`, in reading order.
/// ponytail: a link wrapped onto the next row is two spots, one per row.
pub fn on_screen(buf: &Buffer) -> Vec<Spot> {
    let mut spots: Vec<Spot> = Vec::new();
    for y in buf.area.top()..buf.area.bottom() {
        let mut x = buf.area.left();
        while x < buf.area.right() {
            let symbol = buf[(x, y)].symbol();
            let width = (Span::raw(visible(symbol)).width() as u16).max(1);
            if let Some(url) = url(symbol) {
                match spots.last_mut() {
                    Some(s) if s.y == y && s.x.end == x && s.url == url => s.x.end = x + width,
                    _ => spots.push(Spot {
                        y,
                        x: x..x + width,
                        url: url.to_string(),
                    }),
                }
            }
            x += width;
        }
    }
    spots
}

/// Where a link leads inside the viewer: a file (`None` for the current one), and a
/// heading's anchor in it.
#[derive(Debug, PartialEq)]
pub struct Target {
    pub file: Option<PathBuf>,
    pub anchor: Option<String>,
}

/// Where `url`, a link in the document at `doc`, leads if yamdview can show it: a
/// relative path to a file it has a [`crate::parser`] for, an `#anchor`, or both.
/// URLs with a scheme are left to the terminal.
/// ponytail: no percent-decoding, so `my%20notes.md` won't open; `<my notes.md>` will.
pub fn resolve(url: &str, doc: &Path) -> Option<Target> {
    let scheme = url
        .find(':')
        .is_some_and(|i| !url[..i].contains(['/', '#']));
    if scheme || url.starts_with("//") {
        return None;
    }
    let (path, anchor) = match url.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor.to_string())),
        None => (url, None),
    };
    let path = path.split('?').next().unwrap_or_default();
    if path.is_empty() {
        return anchor.filter(|a| !a.is_empty()).map(|anchor| Target {
            file: None,
            anchor: Some(anchor),
        });
    }
    let file = doc.parent().unwrap_or(Path::new("")).join(path);
    (file.is_file() && crate::parser(&file).is_some()).then(|| Target {
        file: Some(file),
        anchor: anchor.filter(|a| !a.is_empty()),
    })
}

/// The anchor GitHub gives a heading's text: lowercased, punctuation dropped, spaces
/// as dashes. An explicit id (`# Title {#id}`) wins.
pub fn slug(text: &str) -> String {
    let id = text
        .trim_end()
        .strip_suffix('}')
        .and_then(|t| t.rsplit_once(" {"))
        .and_then(|(_, attrs)| attrs.split(' ').find_map(|a| a.strip_prefix('#')));
    if let Some(id) = id {
        return id.to_string();
    }
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Theme, markdown, render};

    #[test]
    fn resolves_relative_links_and_anchors_only() {
        let dir = std::env::temp_dir().join(format!("yamdview-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/guide.md"), "").unwrap();
        std::fs::write(dir.join("logo.png"), "").unwrap();
        let doc = dir.join("README.md");
        let file = |file: &str, anchor: Option<&str>| {
            Some(Target {
                file: Some(dir.join(file)),
                anchor: anchor.map(String::from),
            })
        };
        let found = [
            "docs/guide.md",
            "docs/guide.md#setup",
            "#usage",
            "https://x/guide.md",
            "mailto:a@b",
            "missing.md",
            "logo.png",
            "#",
        ]
        .map(|url| resolve(url, &doc));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found[0], file("docs/guide.md", None));
        assert_eq!(found[1], file("docs/guide.md", Some("setup")));
        assert_eq!(
            found[2],
            Some(Target {
                file: None,
                anchor: Some("usage".into())
            })
        );
        assert_eq!(found[3..], [None, None, None, None, None]);
    }

    #[test]
    fn slugs_like_github() {
        assert_eq!(slug("Getting Started!"), "getting-started");
        assert_eq!(slug("API: v2 & more"), "api-v2--more");
        assert_eq!(slug("Install {#setup .wide}"), "setup");
    }

    #[test]
    fn finds_links_on_screen_across_wide_characters() {
        let mut buf = Buffer::with_lines(["a 猫b c"]);
        for x in 2..5 {
            buf[(x, 0)].underline_color = tag(1);
        }
        buf[(6, 0)].underline_color = tag(2);
        let area = buf.area;
        apply(
            &mut buf,
            area,
            &["one.md".to_string(), "two.md".to_string()],
        );
        let spot = |x, url: &str| Spot {
            y: 0,
            x,
            url: url.into(),
        };
        assert_eq!(
            on_screen(&buf),
            [spot(2..5, "one.md"), spot(6..7, "two.md")]
        );
    }

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
