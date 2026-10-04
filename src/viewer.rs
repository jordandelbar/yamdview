//! The rendered document: laid out in blocks, scrolled, searched, drawn or printed.

use crate::diacritics::DIACRITICS;
use crate::kitty::{image_cells, kitty, png_size, upload};
use crate::search;
use ratatui::{
    Frame,
    backend::IntoCrossterm,
    buffer::Buffer,
    crossterm::{queue, style::PrintStyledContent, terminal::window_size},
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Paragraph, Widget, Wrap},
};
use std::{io::Write, path::PathBuf, time::SystemTime};
use yamdview::{
    Chunk, Theme, boxed, chunks, diagram, document, is_heading, link, markdown, render,
};

/// One buffer row as a line of output: plain text, or styled runs of cells.
/// Trailing blank cells are dropped; a wide character's covered cells are skipped.
pub fn print_row(out: &mut impl Write, buf: &Buffer, y: u16, styled: bool) -> std::io::Result<()> {
    let mut cells = Vec::new();
    let mut x = 0;
    while x < buf.area.width {
        let cell = &buf[(x, y)];
        x += (Span::raw(link::visible(cell.symbol())).width() as u16).max(1);
        cells.push((cell.symbol(), cell.style()));
    }
    let blank = |(symbol, style): &(&str, Style)| {
        *symbol == " " && style.bg.is_none_or(|bg| bg == Color::Reset)
    };
    let end = cells.iter().rposition(|c| !blank(c)).map_or(0, |i| i + 1);
    let mut cells = cells[..end].iter().peekable();
    while let Some(&(symbol, style)) = cells.next() {
        let mut run = symbol.to_string();
        while let Some(&&(next, _)) = cells.peek().filter(|(_, s)| *s == style) {
            run.push_str(next);
            cells.next();
        }
        if styled {
            queue!(out, PrintStyledContent(style.into_crossterm().apply(run)))?;
        } else {
            out.write_all(run.as_bytes())?;
        }
    }
    out.write_all(b"\n")
}

enum Block {
    // u16 height: ratatui scrolls a Paragraph by u16 rows, see `paragraphs`.
    Text(Box<Paragraph<'static>>, u16),
    Image { id: u32, cols: u16, rows: u16 },
}

impl Block {
    pub fn height(&self) -> u32 {
        match self {
            Block::Text(_, h) => u32::from(*h),
            Block::Image { rows, .. } => u32::from(*rows) + 1, // one blank row below each diagram
        }
    }
}

/// The first of `rows` (in order) below `from`, or with `backwards` the last above it.
fn next(rows: &[u32], from: u32, backwards: bool) -> Option<u32> {
    if backwards {
        rows.iter().rev().find(|&&row| row < from).copied()
    } else {
        rows.iter().find(|&&row| row > from).copied()
    }
}

/// A stretch of laid-out text, with the rows (within it) that `[`/`]` and `{`/`}` jump to.
pub struct Laid {
    pub paragraph: Paragraph<'static>,
    pub rows: u16,
    pub headings: Vec<u16>,
    /// Where a paragraph, list, table or box starts: a non-blank line after a blank one.
    pub starts: Vec<u16>,
}

/// Wrap `text` into paragraphs of about 1024 rows, split between source lines (wrapping
/// is per line, so rows add up exactly). ratatui scrolls a Paragraph by u16 rows, and
/// small blocks keep per-frame clones and search layout cheap. Each comes with the
/// rows where the lines matching `heading` start, and where blocks start. `after_blank`
/// says whether the line before `text` was blank, and is left saying it for the last.
pub fn paragraphs(
    text: Text<'static>,
    width: u16,
    heading: impl Fn(&Line) -> bool,
    after_blank: &mut bool,
) -> Vec<Laid> {
    let Text {
        lines,
        style,
        alignment,
    } = text;
    // ponytail: one source line wrapping past 65535 rows (~5 MB at 80 columns) is cut there.
    let clamp = |rows: usize| u16::try_from(rows).unwrap_or(u16::MAX);
    let laid = |lines, rows: usize, headings, starts| Laid {
        paragraph: Paragraph::new(Text {
            lines,
            style,
            alignment,
        })
        .wrap(Wrap { trim: false }),
        rows: clamp(rows),
        headings,
        starts,
    };
    let (mut out, mut chunk, mut rows) = (Vec::new(), Vec::new(), 0);
    let (mut heads, mut starts) = (Vec::new(), Vec::new());
    for line in lines {
        let n = Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(width);
        if rows + n > 1024 && !chunk.is_empty() {
            out.push(laid(
                std::mem::take(&mut chunk),
                rows,
                std::mem::take(&mut heads),
                std::mem::take(&mut starts),
            ));
            rows = 0;
        }
        if heading(&line) {
            heads.push(clamp(rows));
        }
        let blank = line.spans.iter().all(|s| s.content.trim().is_empty());
        if !blank && *after_blank {
            starts.push(clamp(rows));
        }
        *after_blank = blank;
        rows += n;
        chunk.push(line);
    }
    out.push(laid(chunk, rows, heads, starts));
    out
}

/// A terminal cell's size in pixels.
/// ponytail: assumes 10x20px cells if the terminal won't report pixel size.
fn cell_size() -> (u32, u32) {
    window_size()
        .ok()
        .filter(|w| w.width > 0 && w.height > 0 && w.columns > 0 && w.rows > 0)
        .map_or((10, 20), |w| {
            (u32::from(w.width / w.columns), u32::from(w.height / w.rows))
        })
}

pub struct Viewer {
    pub path: PathBuf,
    /// Document rows where headings start, in order, for `[` and `]`.
    headings: Vec<u32>,
    /// Document rows where blocks start (paragraphs, lists, tables, boxes, diagrams),
    /// in order, for `{` and `}`.
    starts: Vec<u32>,
    /// Markdown piped in on stdin. Replaces the file, and there's nothing to watch.
    pub stdin: Option<String>,
    pub mtime: Option<SystemTime>,
    blocks: Vec<Block>,
    pub ids: Vec<u32>,
    pub scroll: u32,
    pub tmux: bool,
    /// Draw diagrams as images; without kitty graphics they show as boxed source.
    pub images: bool,
    /// Link URLs, by the id their text is tagged with; `None` shows each URL after its
    /// link instead, for plain output.
    pub links: Option<Vec<String>>,
    pub theme: Theme,
    pub search: search::Search,
}

impl Viewer {
    pub fn new(
        path: PathBuf,
        stdin: Option<String>,
        tmux: bool,
        images: bool,
        theme: Theme,
    ) -> Self {
        Viewer {
            path,
            headings: Vec::new(),
            starts: Vec::new(),
            stdin,
            mtime: None,
            blocks: Vec::new(),
            ids: Vec::new(),
            scroll: 0,
            tmux,
            images,
            links: Some(Vec::new()),
            theme,
            search: search::Search::default(),
        }
    }

    /// Delete the uploaded diagrams from the terminal.
    pub fn free_images(&mut self, out: &mut impl Write) -> std::io::Result<()> {
        for id in self.ids.drain(..) {
            kitty(out, &format!("a=d,d=I,i={id},q=2"), self.tmux)?;
        }
        Ok(())
    }

    /// The file changed since the last `rebuild`. Never for stdin.
    pub fn changed_on_disk(&self) -> bool {
        self.stdin.is_none()
            && std::fs::metadata(&self.path)
                .and_then(|m| m.modified())
                .ok()
                != self.mtime
    }

    /// The markdown: stdin, or the file read afresh (noting its mtime for `changed_on_disk`).
    fn read(&mut self) -> std::io::Result<String> {
        if let Some(md) = &self.stdin {
            return Ok(md.clone());
        }
        self.mtime = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        std::fs::read_to_string(&self.path)
    }

    /// Upload a rendered diagram and add it as the next block, sized in `cell`s.
    fn push_image(
        &mut self,
        out: &mut impl Write,
        png: &[u8],
        cell: (u32, u32),
    ) -> std::io::Result<()> {
        let max = DIACRITICS.len() as u32;
        let (w, h) = png_size(png);
        let (cols, rows) = (
            w.div_ceil(cell.0).min(max) as u16,
            h.div_ceil(cell.1).min(max) as u16,
        );
        // 24-bit id (sent as a truecolor fg): pid keeps viewers in other panes apart.
        let id = (std::process::id() & 0xffff) << 8 | (self.ids.len() as u32 + 1);
        upload(out, id, png, cols, rows, self.tmux)?;
        self.ids.push(id);
        self.starts.push(self.total());
        self.blocks.push(Block::Image { id, cols, rows });
        Ok(())
    }

    /// Re-read the file (or reuse stdin) and re-render everything for the current terminal size.
    pub fn rebuild(&mut self, out: &mut impl Write, width: u16) -> std::io::Result<()> {
        self.free_images(out)?;
        let md = self.read()?;
        let cell = cell_size();
        self.blocks.clear();
        self.headings.clear();
        self.starts.clear();
        // The document starts after nothing, and a diagram ends with a blank row.
        let mut after_blank = true;
        if let Some(links) = &mut self.links {
            links.clear();
        }
        self.search.clear_layout();
        let mut after_box = false;
        for chunk in chunks(markdown::parse(&md)) {
            let chunk = match chunk {
                Chunk::Mermaid { lang, source } if !self.images => Chunk::Boxed {
                    body: vec![document::Block::Code { lang, code: source }],
                    title: "mermaid".to_string(),
                    alert: None,
                },
                chunk => chunk,
            };
            // Boxes carry a blank line on each side; two in a row share one.
            let prev_box = std::mem::replace(&mut after_box, matches!(chunk, Chunk::Boxed { .. }));
            let text = match chunk {
                Chunk::Prose(blocks) => render(&blocks, &self.theme, &mut self.links),
                Chunk::Boxed { body, title, alert } => {
                    let mut text = boxed(&body, &title, alert, &self.theme, width, &mut self.links);
                    if prev_box {
                        text.lines.remove(0);
                    }
                    text
                }
                Chunk::Mermaid { lang, source } => {
                    match diagram(&source, &self.theme, u32::from(width) * cell.0, cell.1) {
                        Ok(Some(png)) => {
                            self.push_image(out, &png, cell)?;
                            after_blank = true;
                            continue;
                        }
                        // Unsupported or invalid diagram: show the source instead.
                        Ok(None) => {
                            let code = [document::Block::Code { lang, code: source }];
                            render(&code, &self.theme, &mut self.links)
                        }
                        Err(e) => {
                            let code = [document::Block::Code { lang, code: source }];
                            let mut t = render(&code, &self.theme, &mut self.links);
                            t.push_line(Line::from(format!("mermaid render failed: {e}")).red());
                            t
                        }
                    }
                }
            };
            let heading = |line: &Line| is_heading(line, &self.theme);
            for laid in paragraphs(text, width, heading, &mut after_blank) {
                let offset = self.total();
                let rows = |rows: Vec<u16>| rows.into_iter().map(move |r| offset + u32::from(r));
                self.headings.extend(rows(laid.headings));
                self.starts.extend(rows(laid.starts));
                self.blocks
                    .push(Block::Text(Box::new(laid.paragraph), laid.rows));
            }
        }
        if self.search.active() {
            self.cache_search(width);
        }
        self.search.refresh();
        out.flush()
    }

    /// Lay out the text for search on first use; `rebuild` drops the cache.
    pub fn cache_search(&mut self, width: u16) {
        if self.search.cached() {
            return;
        }
        let mut offset = 0;
        for block in &self.blocks {
            if let Block::Text(p, h) = block {
                self.search.cache(p, width, *h, offset);
            }
            offset += block.height();
        }
    }

    /// The first heading below row `from`, or with `backwards` the last one above it.
    pub fn heading(&self, from: u32, backwards: bool) -> Option<u32> {
        next(&self.headings, from, backwards)
    }

    /// The first block start below row `from`, or with `backwards` the last one above it.
    pub fn block(&self, from: u32, backwards: bool) -> Option<u32> {
        next(&self.starts, from, backwards)
    }

    /// Render the whole document once to `out`, for pipes, files and tmux's history:
    /// `styled` keeps colors, attributes and diagrams (as placeholders), else plain text.
    pub fn print(&mut self, out: &mut impl Write, width: u16, styled: bool) -> std::io::Result<()> {
        self.rebuild(out, width)?;
        for block in &self.blocks {
            // Text blocks stay under u16 rows, and images under DIACRITICS.len() + 1.
            let area = Rect::new(0, 0, width, block.height() as u16);
            let mut buf = Buffer::empty(area);
            match block {
                Block::Text(p, _) => Paragraph::clone(p).render(area, &mut buf),
                Block::Image { id, cols, rows } => image_cells(&mut buf, *id, *cols, 0..*rows, 0),
            }
            if let Some(urls) = &self.links {
                link::apply(&mut buf, area, urls);
            }
            for y in 0..area.height {
                print_row(out, &buf, y, styled)?;
            }
        }
        out.flush()
    }

    pub fn total(&self) -> u32 {
        self.blocks.iter().map(Block::height).sum()
    }

    pub fn draw(&self, frame: &mut Frame) {
        let full = frame.area();
        let status = self.search.active();
        let area = Rect {
            height: full.height.saturating_sub(u16::from(status)),
            ..full
        };
        let mut y = -i64::from(self.scroll); // top of the current block, relative to the screen
        for block in &self.blocks {
            let h = i64::from(block.height());
            if y + h > 0 && y < i64::from(area.height) {
                // Both fit in u16 once the block is on screen: skip < h, top < area.height.
                let skip = (-y).max(0) as u16; // rows of this block scrolled off the top
                let top = y.max(0) as u16;
                let rows = (y + h).min(i64::from(area.height)) as u16 - top;
                match block {
                    Block::Text(p, _) => {
                        frame.render_widget(
                            Paragraph::clone(p).scroll((skip, 0)),
                            Rect::new(0, top, area.width, rows),
                        );
                    }
                    // Placeholders make scrolling free: each cell names its own image row.
                    Block::Image {
                        id,
                        cols,
                        rows: img_rows,
                    } => {
                        let rows = skip..(skip + rows).min(*img_rows);
                        image_cells(frame.buffer_mut(), *id, (*cols).min(area.width), rows, top);
                    }
                }
            }
            y += h;
        }
        for (index, hit) in self.search.hits.iter().enumerate() {
            if hit.row < self.scroll || hit.row - self.scroll >= u32::from(area.height) {
                continue;
            }
            let y = (hit.row - self.scroll) as u16;
            for x in hit.start..hit.end.min(area.width) {
                let cell = &mut frame.buffer_mut()[(x, y)];
                cell.set_bg(yamdview::theme::color(self.theme.selection));
                if self.search.current == Some(index) {
                    cell.set_fg(yamdview::theme::color(self.theme.accent));
                }
            }
        }
        if let Some(urls) = &self.links {
            link::apply(frame.buffer_mut(), area, urls);
        }
        if status && full.height > 0 {
            let count = if self.search.hits.is_empty() {
                "no matches".to_string()
            } else {
                format!(
                    "{}/{}",
                    self.search.current.map_or(0, |i| i + 1),
                    self.search.hits.len()
                )
            };
            let prompt = if self.search.query.is_empty() {
                "/".to_string()
            } else {
                format!("/{}  [{}]", self.search.query, count)
            };
            frame.render_widget(
                Paragraph::new(prompt),
                Rect::new(0, full.height - 1, full.width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_skips_images_and_highlights_text_above_status() {
        let mut viewer = Viewer::new(PathBuf::new(), None, false, true, Theme::dracula());
        viewer.blocks = vec![
            Block::Image {
                id: 1,
                cols: 2,
                rows: 3,
            },
            Block::Text(Box::new(Paragraph::new("target target")), 1),
        ];
        viewer.scroll = 3;
        viewer.search.query = "target".into();
        viewer.cache_search(20);
        viewer.search.refresh();
        assert_eq!(viewer.search.hits.len(), 2);
        assert_eq!(viewer.search.jump(0, false), Some(4));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 3)).unwrap();
        terminal.draw(|frame| viewer.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 1)].symbol(), "t");
        assert_eq!(
            buffer[(0, 1)].bg,
            yamdview::theme::color(viewer.theme.selection)
        );
        assert_eq!(buffer[(0, 2)].symbol(), "/");
        viewer.search.cancel();
        assert!(viewer.search.hits.is_empty());
        assert_eq!(viewer.search.current, None);
        assert!(!viewer.search.cached());
    }

    /// A viewer of `md` as if piped in, not laid out yet.
    fn piped(md: &str, images: bool) -> Viewer {
        Viewer::new(
            PathBuf::from("-"),
            Some(md.into()),
            false,
            images,
            Theme::dracula(),
        )
    }

    /// `md` as if piped in, laid out `width` columns wide.
    fn built(md: &str, width: u16, images: bool) -> Viewer {
        let mut v = piped(md, images);
        v.rebuild(&mut Vec::new(), width).unwrap();
        v
    }

    /// The `height` rows `v` draws at `width` columns, trailing blanks trimmed.
    fn rows(v: &Viewer, width: u16, height: u16) -> Vec<String> {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| v.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// Every row of `v`.
    fn screen(v: &Viewer, width: u16) -> Vec<String> {
        rows(v, width, v.total() as u16)
    }

    #[test]
    fn documents_past_u16_rows_split_scroll_and_search() {
        let md: String = (0..70_000).map(|i| format!("row{i}\n\n")).collect();
        let mut viewer = built(&md, 10, true);
        assert!(viewer.total() > u32::from(u16::MAX));
        assert!(viewer.blocks.len() > 1);
        assert!(!viewer.search.cached());

        viewer.search.query = "row69999".into();
        viewer.cache_search(10);
        viewer.search.refresh();
        let row = viewer.search.jump(0, false).unwrap();
        assert!(row > u32::from(u16::MAX));
        viewer.scroll = row;
        assert_eq!(rows(&viewer, 10, 3)[0], "row69999");
    }

    #[test]
    fn block_jumps_land_on_each_paragraph_list_box_and_diagram() {
        let md = "# Title\n\nA paragraph long enough to wrap onto a second row.\n\n- one\n- two\n\n```sh\nls\n```\n\n> [!TIP]\n> boxed\n\nRight above.\n```mermaid\ngraph TD\n  A-->B\n```\n\nLast.\n";
        let viewer = built(md, 30, true);
        let rows = rows(&viewer, 30, viewer.total() as u16);
        let starts: Vec<&str> = viewer
            .starts
            .iter()
            .map(|&row| rows[row as usize].as_str())
            .collect();
        let diagram = starts[6];
        assert!(diagram.starts_with('\u{10EEEE}'), "{starts:?}");
        assert_eq!(
            starts,
            [
                "Title",
                "A paragraph long enough to",
                "- one",
                "╭─ sh ╮",
                "╭─ Tip ─╮",
                "Right above.",
                diagram,
                "Last."
            ],
            "a wrapped paragraph and a tight list are one block each"
        );

        let s = viewer.starts.clone();
        assert_eq!(viewer.block(0, false), Some(s[1]));
        assert_eq!(
            viewer.block(s[1] + 1, false),
            Some(s[2]),
            "from inside a paragraph"
        );
        assert_eq!(viewer.block(s[4], true), Some(s[3]));
        assert_eq!(viewer.block(s[7], false), None, "no wrap-around at the end");
        assert_eq!(viewer.block(0, true), None);
    }

    #[test]
    fn heading_rows_are_relative_to_their_block() {
        let mut lines: Vec<Line> = (0..1500).map(|_| Line::from("x")).collect();
        lines[1100] = Line::from("H");
        let heading = |l: &Line| l.spans.first().is_some_and(|s| s.content == "H");
        let blocks = paragraphs(Text::from(lines), 10, heading, &mut true);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            (blocks[0].rows, blocks[0].headings.as_slice()),
            (1024, &[][..])
        );
        assert_eq!(
            blocks[1].headings,
            [1100 - 1024],
            "offset within the second block"
        );
    }

    #[test]
    fn single_line_past_u16_rows_is_cut_off() {
        let blocks = paragraphs(Text::from("x ".repeat(70_000)), 1, |_| false, &mut true);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].rows, u16::MAX);
    }

    #[test]
    fn consecutive_boxes_share_one_blank_line() {
        let md = "Intro.\n\n> [!NOTE]\n> a\n\n> [!TIP]\n> b\n\nOutro.\n";
        let rows = screen(&built(md, 40, true), 40);
        let first = |c: char| rows.iter().position(|r| r.starts_with(c)).unwrap();
        let tip_top = rows.iter().position(|r| r.starts_with("╭─ Tip")).unwrap();
        assert_eq!(
            rows[first('╭') - 1],
            "",
            "blank line above the first box: {rows:#?}"
        );
        assert_eq!(
            tip_top - first('╰'),
            2,
            "one blank line between the boxes: {rows:#?}"
        );
        assert_eq!(
            rows[rows.len() - 2],
            "",
            "blank line before the outro: {rows:#?}"
        );
    }

    #[test]
    fn renders_stdin_without_a_file() {
        let viewer = built("# Piped\n\nfrom a pipe\n", 30, true);
        assert_eq!(viewer.mtime, None, "nothing to watch");
        assert!(!viewer.changed_on_disk());
        let rows = rows(&viewer, 30, 4);
        assert_eq!(rows[0], "Piped");
        assert!(rows.contains(&"from a pipe".to_string()), "{rows:#?}");
    }

    #[test]
    fn reads_the_file_and_notices_saves() {
        let path = std::env::temp_dir().join(format!("yamdview-file-{}.md", std::process::id()));
        std::fs::write(&path, "# On disk\n").unwrap();
        let mut viewer = Viewer::new(path.clone(), None, false, true, Theme::dracula());
        viewer.rebuild(&mut Vec::new(), 20).unwrap();
        assert_eq!(rows(&viewer, 20, 1), ["On disk"]);
        assert!(!viewer.changed_on_disk());
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH).unwrap();
        let changed = viewer.changed_on_disk();
        std::fs::remove_file(&path).unwrap();
        assert!(changed, "a new mtime counts as a save");
    }

    #[test]
    fn heading_rows_match_the_screen_and_jumps_move_between_them() {
        let long = "word ".repeat(30);
        let md = format!(
            "# Top\n\n{long}\n\n## Code\n\n```sh\n# not a heading\n```\n\n## Table\n\n| a |\n| - |\n| b |\n\n### Last\n\nend\n"
        );
        let viewer = built(&md, 30, true);
        let rows = screen(&viewer, 30);
        let titles: Vec<&str> = viewer
            .headings
            .iter()
            .map(|&y| rows[y as usize].as_str())
            .collect();
        assert_eq!(
            titles,
            ["Top", "Code", "Table", "Last"],
            "rows {:?}",
            viewer.headings
        );

        let h = viewer.headings.clone();
        assert_eq!(
            viewer.heading(0, false),
            Some(h[1]),
            "`]` from the top skips the heading already there"
        );
        assert_eq!(viewer.heading(h[1], false), Some(h[2]));
        assert_eq!(viewer.heading(h[2], true), Some(h[1]));
        assert_eq!(
            viewer.heading(h[3], false),
            None,
            "no wrap-around at the end"
        );
        assert_eq!(viewer.heading(0, true), None, "none above the first");
    }

    #[test]
    fn without_images_diagrams_show_as_boxed_source() {
        let viewer = built(
            "Intro.\n\n```mermaid\ngraph TD\n  A-->B\n```\n\nOutro.\n",
            40,
            false,
        );
        assert!(viewer.ids.is_empty(), "nothing uploaded to the terminal");
        assert!(viewer.blocks.iter().all(|b| matches!(b, Block::Text(..))));
        let rows = screen(&viewer, 40);
        assert!(
            rows.iter().any(|r| r.starts_with("╭─ mermaid")),
            "{rows:#?}"
        );
        assert!(rows.iter().any(|r| r.contains("A-->B")), "{rows:#?}");
    }

    fn printed(md: &str, images: bool, styled: bool) -> String {
        let mut viewer = piped(md, images);
        let mut out = Vec::new();
        viewer.print(&mut out, 40, styled).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn prints_plain_text_for_pipes() {
        let out = printed(
            "# Title\n\n- [x] done\n\n猫 wide\n\n```sh\nls\n```\n",
            false,
            false,
        );
        assert!(!out.contains('\x1b'), "no escape codes: {out:?}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "Title");
        assert!(lines.contains(&"[✓] done"), "{lines:#?}");
        assert!(
            lines.contains(&"猫 wide"),
            "wide characters print once: {lines:#?}"
        );
        assert!(lines.iter().any(|l| l.starts_with("╭─ sh")), "{lines:#?}");
        assert!(
            lines.iter().all(|l| !l.ends_with(' ')),
            "trailing blanks trimmed: {lines:#?}"
        );
    }

    #[test]
    fn links_are_hyperlinks_on_screen_and_in_styled_print_only() {
        let md = "See [the docs](https://example.com).\n";
        let viewer = built(md, 40, true);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 1)).unwrap();
        terminal.draw(|frame| viewer.draw(frame)).unwrap();
        let cell = &terminal.backend().buffer()[(4, 0)];
        assert_eq!(
            cell.symbol(),
            "\x1b]8;;https://example.com\x1b\\t\x1b]8;;\x1b\\"
        );

        let styled = printed(md, false, true);
        assert!(
            styled.contains("\x1b]8;;https://example.com\x1b\\"),
            "{styled:?}"
        );
        assert!(
            !styled.contains("(https"),
            "no URL after the link: {styled:?}"
        );

        let mut plain = piped(md, false);
        plain.links = None;
        let mut out = Vec::new();
        plain.print(&mut out, 40, false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "See the docs (https://example.com).\n"
        );
    }

    #[test]
    fn prints_styles_and_diagrams_for_the_terminal() {
        let styled = printed("# Title\n\ntext\n", false, true);
        assert!(
            styled.contains("\x1b[") && styled.contains("Title"),
            "{styled:?}"
        );

        let md = "Intro.\n\n```mermaid\ngraph TD\n  A-->B\n```\n";
        let with_images = printed(md, true, true);
        assert!(with_images.contains("\x1b_G"), "the diagram is uploaded");
        assert!(
            with_images.contains('\u{10EEEE}'),
            "and printed as placeholder cells"
        );
        let without = printed(md, false, false);
        assert!(
            !without.contains('\u{10EEEE}') && without.contains("A-->B"),
            "{without}"
        );
    }
}
