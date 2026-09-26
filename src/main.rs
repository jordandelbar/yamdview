mod diacritics;
mod search;
mod selection;

use base64::{Engine, engine::general_purpose::STANDARD};
use diacritics::DIACRITICS;
use ratatui::{
    Frame,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers,
            MouseButton, MouseEventKind,
        },
        execute,
        terminal::window_size,
    },
    layout::Rect,
    style::{Color, Stylize},
    text::{Line, Text},
    widgets::{Paragraph, Wrap},
};
use std::{
    io::{IsTerminal, Read, Write},
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime},
};
use yamdview::{Chunk, Theme, boxed, diagram, markdown, split};

/// Kitty graphics escape, wrapped for tmux passthrough (`allow-passthrough on`) when needed.
fn kitty(out: &mut impl Write, body: &str, tmux: bool) -> std::io::Result<()> {
    let seq = format!("\x1b_G{body}\x1b\\");
    if tmux {
        write!(out, "\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
    } else {
        out.write_all(seq.as_bytes())
    }
}

/// Upload a PNG as a virtual placement (U=1) of `cols` x `rows` cells. Nothing is drawn
/// until placeholder cells with this id appear on screen; re-uploading an id replaces it.
fn upload(
    out: &mut impl Write,
    id: u32,
    png: &[u8],
    cols: u16,
    rows: u16,
    tmux: bool,
) -> std::io::Result<()> {
    let b64 = STANDARD.encode(png);
    let parts: Vec<&str> = b64
        .as_bytes()
        .chunks(4096)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    for (i, part) in parts.iter().enumerate() {
        let more = u8::from(i + 1 < parts.len());
        let head = if i == 0 {
            format!("f=100,a=T,U=1,i={id},c={cols},r={rows},")
        } else {
            String::new()
        };
        // q=2: no replies, they would land in our input.
        kitty(out, &format!("{head}q=2,m={more};{part}"), tmux)?;
    }
    Ok(())
}

/// Placeholder cell for image row `r`, column `c`. Its fg color carries the image id.
fn placeholder(r: usize, c: usize) -> String {
    format!("\u{10EEEE}{}{}", DIACRITICS[r], DIACRITICS[c])
}

/// Width and height from the PNG IHDR chunk.
fn png_size(png: &[u8]) -> (u32, u32) {
    let be = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
    (be(16), be(20))
}

enum Block {
    // u16 height: ratatui scrolls a Paragraph by u16 rows, see `paragraphs`.
    Text(Box<Paragraph<'static>>, u16),
    Image { id: u32, cols: u16, rows: u16 },
}

impl Block {
    fn height(&self) -> u32 {
        match self {
            Block::Text(_, h) => u32::from(*h),
            Block::Image { rows, .. } => u32::from(*rows) + 1, // one blank row below each diagram
        }
    }
}

/// Wrap `text` into paragraphs of about 1024 rows, split between source lines (wrapping
/// is per line, so rows add up exactly). ratatui scrolls a Paragraph by u16 rows, and
/// small blocks keep per-frame clones and search layout cheap.
fn paragraphs(text: Text<'static>, width: u16) -> Vec<(Paragraph<'static>, u16)> {
    let Text {
        lines,
        style,
        alignment,
    } = text;
    // ponytail: one source line wrapping past 65535 rows (~5 MB at 80 columns) is cut there.
    let block = |lines, rows: usize| {
        let p = Paragraph::new(Text {
            lines,
            style,
            alignment,
        })
        .wrap(Wrap { trim: false });
        (p, u16::try_from(rows).unwrap_or(u16::MAX))
    };
    let (mut out, mut chunk, mut rows) = (Vec::new(), Vec::new(), 0);
    for line in lines {
        let n = Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(width);
        if rows + n > 1024 && !chunk.is_empty() {
            out.push(block(std::mem::take(&mut chunk), rows));
            rows = 0;
        }
        rows += n;
        chunk.push(line);
    }
    out.push(block(chunk, rows));
    out
}

struct Viewer {
    path: PathBuf,
    /// Markdown piped in on stdin. Replaces the file, and there's nothing to watch.
    stdin: Option<String>,
    mtime: Option<SystemTime>,
    blocks: Vec<Block>,
    ids: Vec<u32>,
    scroll: u32,
    tmux: bool,
    theme: Theme,
    search: search::Search,
}

impl Viewer {
    /// Re-read the file (or reuse stdin) and re-render everything for the current terminal size.
    fn rebuild(&mut self, out: &mut impl Write, width: u16) -> std::io::Result<()> {
        for id in self.ids.drain(..) {
            kitty(out, &format!("a=d,d=I,i={id},q=2"), self.tmux)?;
        }
        let md = match &self.stdin {
            Some(md) => md.clone(),
            None => {
                self.mtime = std::fs::metadata(&self.path)
                    .and_then(|m| m.modified())
                    .ok();
                std::fs::read_to_string(&self.path)?
            }
        };

        // ponytail: assumes 10x20px cells if the terminal won't report pixel size.
        let cell = window_size()
            .ok()
            .filter(|w| w.width > 0 && w.height > 0 && w.columns > 0 && w.rows > 0)
            .map_or((10, 20), |w| {
                (u32::from(w.width / w.columns), u32::from(w.height / w.rows))
            });
        let max = DIACRITICS.len() as u32;

        self.blocks.clear();
        self.search.clear_layout();
        let mut after_box = false;
        for chunk in split(&md) {
            if matches!(chunk, Chunk::Text(t) if t.trim().is_empty()) {
                continue;
            }
            // Boxes carry a blank line on each side; two in a row share one.
            let prev_box = std::mem::replace(&mut after_box, matches!(chunk, Chunk::Boxed { .. }));
            let text = match chunk {
                Chunk::Text(t) => markdown(t, &self.theme),
                Chunk::Boxed { md, title, alert } => {
                    let mut text = boxed(&md, &title, alert, &self.theme, width);
                    if prev_box {
                        text.lines.remove(0);
                    }
                    text
                }
                Chunk::Mermaid { raw, source } => {
                    match diagram(&source, &self.theme, u32::from(width) * cell.0, cell.1) {
                        Ok(Some(png)) => {
                            let (w, h) = png_size(&png);
                            let (cols, rows) = (
                                w.div_ceil(cell.0).min(max) as u16,
                                h.div_ceil(cell.1).min(max) as u16,
                            );
                            // 24-bit id (sent as a truecolor fg): pid keeps viewers in other panes apart.
                            let id =
                                (std::process::id() & 0xffff) << 8 | (self.ids.len() as u32 + 1);
                            upload(out, id, &png, cols, rows, self.tmux)?;
                            self.ids.push(id);
                            self.blocks.push(Block::Image { id, cols, rows });
                            continue;
                        }
                        // Unsupported or invalid diagram: show the source instead.
                        Ok(None) => markdown(raw, &self.theme),
                        Err(e) => {
                            let mut t = markdown(raw, &self.theme);
                            t.push_line(Line::from(format!("mermaid render failed: {e}")).red());
                            t
                        }
                    }
                }
            };
            for (p, h) in paragraphs(text, width) {
                self.blocks.push(Block::Text(Box::new(p), h));
            }
        }
        if self.search.editing || !self.search.query.is_empty() {
            self.cache_search(width);
        }
        self.search.refresh();
        out.flush()
    }

    /// Lay out the text for search on first use; `rebuild` drops the cache.
    fn cache_search(&mut self, width: u16) {
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

    fn total(&self) -> u32 {
        self.blocks.iter().map(Block::height).sum()
    }

    fn draw(&self, frame: &mut Frame) {
        let full = frame.area();
        let status = self.search.editing || !self.search.query.is_empty();
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
                        let fg = Color::Rgb((id >> 16) as u8, (id >> 8) as u8, *id as u8);
                        let buf = frame.buffer_mut();
                        for r in skip..(skip + rows).min(*img_rows) {
                            for c in 0..(*cols).min(area.width) {
                                buf[(c, top + r - skip)]
                                    .set_symbol(&placeholder(r.into(), c.into()))
                                    .set_fg(fg);
                            }
                        }
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tmux = std::env::var_os("TMUX").is_some();
    let mut mouse = true;
    let mut path = None;
    let mut read_stdin = false;
    let mut positional = false;
    for arg in std::env::args_os().skip(1) {
        if !positional && arg == "--" {
            positional = true;
        } else if !positional && arg == "--mouse" {
            mouse = true;
        } else if !positional && arg == "--no-mouse" {
            mouse = false;
        } else if !positional && arg == "-" && path.is_none() && !read_stdin {
            read_stdin = true;
        } else if !positional && arg.to_string_lossy().starts_with('-') && arg != "-" {
            return Err(format!("unknown option: {}", arg.to_string_lossy()).into());
        } else if path.is_none() && !read_stdin {
            path = Some(PathBuf::from(arg));
        } else {
            return Err("usage: yamdview [--mouse|--no-mouse] [--] [FILE | -]".into());
        }
    }
    // `-`, or a pipe with no file given: read stdin now, before the TUI takes over.
    // Keys still work: crossterm reads them from /dev/tty when stdin isn't a terminal.
    let stdin = if read_stdin || (path.is_none() && !std::io::stdin().is_terminal()) {
        let mut md = String::new();
        std::io::stdin().read_to_string(&mut md)?;
        Some(md)
    } else {
        None
    };
    let path =
        path.unwrap_or_else(|| PathBuf::from(if stdin.is_some() { "-" } else { "README.md" }));
    let mut v = Viewer {
        path,
        stdin,
        mtime: None,
        blocks: Vec::new(),
        ids: Vec::new(),
        scroll: 0,
        tmux,
        theme: Theme::detect(),
        search: search::Search::default(),
    };
    let mut terminal = ratatui::init();

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        if mouse {
            execute!(std::io::stdout(), EnableMouseCapture)?;
        }
        let mut selection = selection::Selection::default();
        let (mut dirty, mut repaint) = (true, false);
        loop {
            let size = terminal.size()?;
            if dirty {
                selection.clear();
                v.rebuild(terminal.backend_mut(), size.width)?;
                terminal.clear()?;
                (dirty, repaint) = (false, true);
            }
            let height = size
                .height
                .saturating_sub(u16::from(v.search.editing || !v.search.query.is_empty()));
            let (page, max) = (
                u32::from(height.saturating_sub(2)),
                v.total().saturating_sub(u32::from(height)),
            );
            v.scroll = v.scroll.min(max);
            let rendered = terminal
                .draw(|f| {
                    v.draw(f);
                    selection.highlight(f.buffer_mut(), yamdview::theme::color(v.theme.selection));
                })?
                .buffer
                .clone();
            // Ghostty <= 1.3.1 loses the placeholders' row diacritics when they arrive as
            // incremental tmux pane updates, so every row shows image row 0. A full client
            // repaint fixes it, but tmux reads our output asynchronously: repainting right
            // after drawing can beat our frame and change nothing. So repaint once the view
            // has been still for 50ms, which also debounces held keys.
            // ponytail: drop once Ghostty handles incremental placeholder updates.
            let settle = repaint && v.tmux;
            // Otherwise poll so a save in the editor shows up within a quarter second.
            let timeout = Duration::from_millis(if settle { 50 } else { 250 });
            if !event::poll(timeout)? {
                if settle {
                    Command::new("tmux").arg("refresh-client").status()?;
                    repaint = false;
                }
            } else {
                let s = v.scroll;
                let mut search_jump = false;
                v.scroll = match event::read()? {
                    event::Event::Key(k) if k.kind == KeyEventKind::Press => {
                        selection.clear();
                        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                        if v.search.editing && !(ctrl && k.code == KeyCode::Char('c')) {
                            match k.code {
                                KeyCode::Enter => v.search.editing = false,
                                KeyCode::Esc => {
                                    v.scroll = v.search.cancel();
                                }
                                KeyCode::Backspace => {
                                    v.search.query.pop();
                                }
                                KeyCode::Char(c)
                                    if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) =>
                                {
                                    v.search.query.push(c)
                                }
                                _ => {}
                            }
                            if !matches!(k.code, KeyCode::Enter | KeyCode::Esc) {
                                v.scroll = v.search.incremental();
                            }
                            repaint = true;
                            continue;
                        }
                        match k.code {
                            KeyCode::Char('/') => {
                                repaint = true;
                                v.cache_search(size.width);
                                v.search.begin(s);
                                s
                            }
                            KeyCode::Char('n') => {
                                search_jump = true;
                                v.search.jump(s, false).unwrap_or(s)
                            }
                            KeyCode::Char('N') => {
                                search_jump = true;
                                v.search.jump(s, true).unwrap_or(s)
                            }
                            KeyCode::Esc if !v.search.query.is_empty() => {
                                repaint = true;
                                v.search.cancel();
                                s
                            }
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Char('c') if ctrl => return Ok(()),
                            KeyCode::Char('d') if ctrl => s.saturating_add(page / 2),
                            KeyCode::Char('u') if ctrl => s.saturating_sub(page / 2),
                            KeyCode::Char('j') | KeyCode::Down => s.saturating_add(1),
                            KeyCode::Char('k') | KeyCode::Up => s.saturating_sub(1),
                            KeyCode::Char(' ') | KeyCode::PageDown => s.saturating_add(page),
                            KeyCode::Char('b') | KeyCode::PageUp => s.saturating_sub(page),
                            KeyCode::Char('g') | KeyCode::Home => 0,
                            KeyCode::Char('G') | KeyCode::End => max,
                            _ => s,
                        }
                    }
                    event::Event::Mouse(m) => {
                        // Keep selections off the search status line.
                        let row = m.row.min(height.saturating_sub(1));
                        match m.kind {
                            MouseEventKind::ScrollDown => {
                                selection.clear();
                                s.saturating_add(3)
                            }
                            MouseEventKind::ScrollUp => {
                                selection.clear();
                                s.saturating_sub(3)
                            }
                            MouseEventKind::Down(MouseButton::Left) if v.tmux => {
                                selection.start(m.column, row);
                                repaint = true;
                                s
                            }
                            MouseEventKind::Drag(MouseButton::Left) if v.tmux => {
                                selection.drag(m.column, row);
                                repaint = true;
                                s
                            }
                            MouseEventKind::Up(MouseButton::Left) if v.tmux => {
                                selection.drag(m.column, row);
                                // Best effort: a failed copy (old tmux, no server) must not quit the viewer.
                                let _ = selection::copy_to_tmux(&selection.text(&rendered));
                                selection.clear();
                                repaint = true;
                                s
                            }
                            _ => s,
                        }
                    }
                    event::Event::Resize(..) => {
                        dirty = true;
                        s
                    }
                    _ => s,
                }
                .min(max);
                if v.scroll != s && !search_jump {
                    v.search.current = None;
                }
                repaint |= v.scroll != s;
            }
            if v.stdin.is_none() {
                let mtime = std::fs::metadata(&v.path).and_then(|m| m.modified()).ok();
                dirty |= mtime != v.mtime;
            }
        }
    })();
    // Free the uploaded images, then hand the terminal back.
    for id in &v.ids {
        kitty(
            terminal.backend_mut(),
            &format!("a=d,d=I,i={id},q=2"),
            v.tmux,
        )?;
    }
    execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_is_one_cell_wide() {
        // ratatui lays cells out by display width; anything but 1 would shear the image.
        assert_eq!(Line::from(placeholder(3, 7)).width(), 1);
    }

    #[test]
    fn search_skips_images_and_highlights_text_above_status() {
        let mut viewer = Viewer {
            path: PathBuf::new(),
            stdin: None,
            mtime: None,
            ids: Vec::new(),
            blocks: vec![
                Block::Image {
                    id: 1,
                    cols: 2,
                    rows: 3,
                },
                Block::Text(Box::new(Paragraph::new("target target")), 1),
            ],
            scroll: 3,
            tmux: false,
            theme: Theme::dracula(),
            search: search::Search::default(),
        };
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

    #[test]
    fn documents_past_u16_rows_split_scroll_and_search() {
        let path = std::env::temp_dir().join(format!("yamdview-test-{}.md", std::process::id()));
        let md: String = (0..70_000).map(|i| format!("row{i}\n\n")).collect();
        std::fs::write(&path, md).unwrap();
        let mut viewer = Viewer {
            path: path.clone(),
            stdin: None,
            mtime: None,
            ids: Vec::new(),
            blocks: Vec::new(),
            scroll: 0,
            tmux: false,
            theme: Theme::dracula(),
            search: search::Search::default(),
        };
        viewer.rebuild(&mut Vec::new(), 10).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(viewer.total() > u32::from(u16::MAX));
        assert!(viewer.blocks.len() > 1);
        assert!(!viewer.search.cached());

        viewer.search.query = "row69999".into();
        viewer.cache_search(10);
        viewer.search.refresh();
        let row = viewer.search.jump(0, false).unwrap();
        assert!(row > u32::from(u16::MAX));
        viewer.scroll = row;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(10, 3)).unwrap();
        terminal.draw(|frame| viewer.draw(frame)).unwrap();
        let top: String = (0..8)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_string())
            .collect();
        assert_eq!(top, "row69999");
    }

    #[test]
    fn single_line_past_u16_rows_is_cut_off() {
        let blocks = paragraphs(Text::from("x ".repeat(70_000)), 1);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].1, u16::MAX);
    }

    #[test]
    fn consecutive_boxes_share_one_blank_line() {
        let path = std::env::temp_dir().join(format!("yamdview-boxes-{}.md", std::process::id()));
        std::fs::write(
            &path,
            "Intro.\n\n> [!NOTE]\n> a\n\n> [!TIP]\n> b\n\nOutro.\n",
        )
        .unwrap();
        let mut viewer = Viewer {
            path: path.clone(),
            stdin: None,
            mtime: None,
            ids: Vec::new(),
            blocks: Vec::new(),
            scroll: 0,
            tmux: false,
            theme: Theme::dracula(),
            search: search::Search::default(),
        };
        viewer.rebuild(&mut Vec::new(), 40).unwrap();
        std::fs::remove_file(&path).unwrap();
        let height = viewer.total() as u16;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, height)).unwrap();
        terminal.draw(|frame| viewer.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..height)
            .map(|y| {
                (0..40)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
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
        let mut viewer = Viewer {
            path: PathBuf::from("-"),
            stdin: Some("# Piped\n\nfrom a pipe\n".into()),
            mtime: None,
            ids: Vec::new(),
            blocks: Vec::new(),
            scroll: 0,
            tmux: false,
            theme: Theme::dracula(),
            search: search::Search::default(),
        };
        viewer.rebuild(&mut Vec::new(), 30).unwrap();
        assert_eq!(viewer.mtime, None, "nothing to watch");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 4)).unwrap();
        terminal.draw(|frame| viewer.draw(frame)).unwrap();
        let rows: Vec<String> = (0..4)
            .map(|y| {
                (0..30)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        assert_eq!(rows[0], "Piped");
        assert!(rows.contains(&"from a pipe".to_string()), "{rows:#?}");
    }
}
