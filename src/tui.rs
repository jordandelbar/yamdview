//! The interactive viewer: the event loop, keys and mouse.

use crate::kitty::{pointer, refresh_tmux};
use crate::selection::{self, Selection};
use crate::viewer::Viewer;
use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
            KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        },
        execute,
    },
};
use std::{error::Error, time::Duration};
use yamdview::link;

/// Where a key or mouse event moves the view.
enum Move {
    /// Scroll to this row, leaving the current search hit behind if it moves.
    To(u32),
    /// Scroll to this row and keep the current search hit: `n`, `N`, typing a query.
    Jump(u32),
    /// Another file was opened (or gone back to): lay it out, then scroll where the
    /// viewer says.
    Load,
    Quit,
}

struct Tui {
    v: Viewer,
    selection: Selection,
    /// Re-read and re-render the document: on start, resize, or a save.
    dirty: bool,
    /// The screen changed since the last tmux repaint.
    repaint: bool,
    /// The link `Tab` picked, by its document row and first column.
    focus: Option<(u32, u16)>,
    /// Where the left button went down, to tell a click from a drag.
    down: Option<(u16, u16)>,
    /// Where the mouse is on screen, to highlight the link under it.
    hover: Option<(u16, u16)>,
    /// The pointer is a hand, over a link.
    hand: bool,
}

/// Take over the terminal and show `v` until the user quits.
pub fn run(v: Viewer, mouse: bool) -> Result<(), Box<dyn Error>> {
    let mut terminal = ratatui::init();
    let mut tui = Tui {
        v,
        selection: Selection::default(),
        dirty: true,
        repaint: false,
        focus: None,
        down: None,
        hover: None,
        hand: false,
    };
    let result = tui.event_loop(&mut terminal, mouse);
    // Free the uploaded images, then hand the terminal back.
    tui.v.free_images(terminal.backend_mut())?;
    if tui.hand {
        pointer(&mut std::io::stdout(), "default", tui.v.tmux)?;
    }
    execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

/// The link on screen in `buf` covering the cell at `(x, y)`.
fn spot_at(buf: &Buffer, (x, y): (u16, u16)) -> Option<link::Spot> {
    link::on_screen(buf)
        .into_iter()
        .find(|s| s.y == y && s.x.contains(&x))
}

impl Tui {
    fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        mouse: bool,
    ) -> Result<(), Box<dyn Error>> {
        if mouse {
            execute!(std::io::stdout(), EnableMouseCapture)?;
        }
        loop {
            let size = terminal.size()?;
            if self.dirty {
                self.selection.clear();
                self.focus = None;
                self.v.rebuild(terminal.backend_mut(), size.width)?;
                terminal.clear()?;
                (self.dirty, self.repaint) = (false, true);
            }
            let height = size
                .height
                .saturating_sub(u16::from(self.v.search.active()));
            let (page, max) = (
                u32::from(height.saturating_sub(2)),
                self.v.total().saturating_sub(u32::from(height)),
            );
            self.v.scroll = self.v.scroll.min(max);
            let rendered = terminal
                .draw(|f| {
                    self.v.draw(f);
                    let color = yamdview::theme::color(self.v.theme.selection);
                    self.selection.highlight(f.buffer_mut(), color);
                    let hovered = self.hover.and_then(|at| spot_at(f.buffer_mut(), at));
                    for spot in self.focused(f.buffer_mut()).into_iter().chain(hovered) {
                        for x in spot.x {
                            f.buffer_mut()[(x, spot.y)].set_bg(color);
                        }
                    }
                })?
                .buffer
                .clone();
            // A hand over links, checked after each frame: scrolling moves text under it too.
            let hand = self.hover.and_then(|at| spot_at(&rendered, at)).is_some();
            if hand != self.hand {
                let shape = if hand { "pointer" } else { "default" };
                pointer(&mut std::io::stdout(), shape, self.v.tmux)?;
                self.hand = hand;
            }
            // tmux reads our output asynchronously: a repaint right after drawing can beat
            // our frame and change nothing. So repaint once the view has been still for
            // 50ms, which also debounces held keys.
            let settle = self.repaint && self.v.tmux;
            // Otherwise poll so a save in the editor shows up within a quarter second.
            let timeout = Duration::from_millis(if settle { 50 } else { 250 });
            if !event::poll(timeout)? {
                if settle {
                    refresh_tmux();
                    self.repaint = false;
                }
            } else {
                let s = self.v.scroll;
                let step = match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        self.key(k, page, max, size.width, &rendered)
                    }
                    Event::Mouse(m) => self.mouse(m, height, &rendered),
                    Event::Resize(..) => {
                        self.dirty = true;
                        Move::To(s)
                    }
                    _ => Move::To(s),
                };
                let (row, jump) = match step {
                    Move::To(row) => (row, false),
                    Move::Jump(row) => (row, true),
                    Move::Load => {
                        (self.dirty, self.focus) = (true, None);
                        continue;
                    }
                    Move::Quit => return Ok(()),
                };
                self.v.scroll = row.min(max);
                if self.v.scroll != s && !jump {
                    self.v.search.current = None;
                }
                self.repaint |= self.v.scroll != s;
            }
            self.dirty |= self.v.changed_on_disk();
        }
    }

    /// The focused link, if it's on screen in `buf`.
    fn focused(&self, buf: &Buffer) -> Option<link::Spot> {
        let (row, x) = self.focus?;
        let y = u16::try_from(row.checked_sub(self.v.scroll)?).ok()?;
        spot_at(buf, (x, y))
    }

    /// Focus the next link on screen after the focused one, or with `backwards` the one
    /// before, wrapping around.
    fn tab(&mut self, rendered: &Buffer, backwards: bool) {
        let scroll = self.v.scroll;
        let at = |s: &link::Spot| (scroll + u32::from(s.y), s.x.start);
        let mut spots: Vec<_> = link::on_screen(rendered).iter().map(at).collect();
        if backwards {
            spots.reverse();
        }
        let after = |&&p: &&(u32, u16)| match self.focus {
            None => true,
            Some(f) if backwards => p < f,
            Some(f) => p > f,
        };
        self.focus = spots.iter().find(after).or(spots.first()).copied();
        self.repaint = true;
    }

    /// Follow a link, or go back with `None`: scroll here, or load another file.
    fn go(&mut self, url: Option<&str>) -> Move {
        let s = self.v.scroll;
        let moved = match url {
            Some(url) => self.v.follow(url),
            None => self.v.back(),
        };
        match moved {
            Some(true) => Move::Load,
            Some(false) => {
                self.focus = None;
                Move::To(self.v.scroll)
            }
            None => Move::To(s),
        }
    }

    fn key(&mut self, k: KeyEvent, page: u32, max: u32, width: u16, rendered: &Buffer) -> Move {
        self.selection.clear();
        let v = &mut self.v;
        let s = v.scroll;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if v.search.editing && !(ctrl && k.code == KeyCode::Char('c')) {
            self.repaint = true;
            return Move::Jump(match k.code {
                KeyCode::Enter => {
                    v.search.editing = false;
                    s
                }
                KeyCode::Esc => v.search.cancel(),
                KeyCode::Backspace => {
                    v.search.query.pop();
                    v.search.incremental()
                }
                KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                    v.search.query.push(c);
                    v.search.incremental()
                }
                _ => v.search.incremental(),
            });
        }
        Move::To(match k.code {
            KeyCode::Char('/') => {
                self.repaint = true;
                v.cache_search(width);
                v.search.begin(s);
                s
            }
            KeyCode::Char(']') => v.heading(s, false).unwrap_or(s),
            KeyCode::Char('[') => v.heading(s, true).unwrap_or(s),
            KeyCode::Char('}') => v.block(s, false).unwrap_or(s),
            KeyCode::Char('{') => v.block(s, true).unwrap_or(s),
            KeyCode::Tab => {
                self.tab(rendered, false);
                s
            }
            KeyCode::BackTab => {
                self.tab(rendered, true);
                s
            }
            KeyCode::Enter => {
                let url = self.focused(rendered).map(|spot| spot.url);
                return match url {
                    Some(url) => self.go(Some(&url)),
                    None => Move::To(s),
                };
            }
            KeyCode::Backspace => return self.go(None),
            KeyCode::Char('n') => return Move::Jump(v.search.jump(s, false).unwrap_or(s)),
            KeyCode::Char('N') => return Move::Jump(v.search.jump(s, true).unwrap_or(s)),
            KeyCode::Esc if self.focus.is_some() => {
                self.focus = None;
                self.repaint = true;
                s
            }
            KeyCode::Esc if !v.search.query.is_empty() => {
                self.repaint = true;
                v.search.cancel();
                s
            }
            KeyCode::Char('q') | KeyCode::Esc => return Move::Quit,
            KeyCode::Char('c') if ctrl => return Move::Quit,
            KeyCode::Char('d') if ctrl => s.saturating_add(page / 2),
            KeyCode::Char('u') if ctrl => s.saturating_sub(page / 2),
            KeyCode::Char('j') | KeyCode::Down => s.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => s.saturating_sub(1),
            KeyCode::Char(' ') | KeyCode::PageDown => s.saturating_add(page),
            KeyCode::Char('b') | KeyCode::PageUp => s.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => 0,
            KeyCode::Char('G') | KeyCode::End => max,
            _ => s,
        })
    }

    /// Wheel scrolling, clicking links, and in tmux, selecting text to copy.
    fn mouse(&mut self, m: MouseEvent, height: u16, rendered: &Buffer) -> Move {
        let s = self.v.scroll;
        // Keep selections off the search status line.
        let row = m.row.min(height.saturating_sub(1));
        let tmux = self.v.tmux;
        match m.kind {
            MouseEventKind::ScrollDown => {
                self.selection.clear();
                return Move::To(s.saturating_add(3));
            }
            MouseEventKind::ScrollUp => {
                self.selection.clear();
                return Move::To(s.saturating_sub(3));
            }
            MouseEventKind::Moved => {
                let at = (m.row < height).then_some((m.column, m.row));
                // Redraw when entering, leaving or moving over a link, not over plain text.
                let link = |at: Option<(u16, u16)>| at.and_then(|at| spot_at(rendered, at));
                if link(self.hover).is_some() || link(at).is_some() {
                    self.repaint = true;
                }
                self.hover = at;
                return Move::To(s);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.down = Some((m.column, row));
                if tmux {
                    self.selection.start(m.column, row);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if tmux => self.selection.drag(m.column, row),
            MouseEventKind::Up(MouseButton::Left) => {
                if tmux {
                    self.selection.drag(m.column, row);
                    // Best effort: a failed copy (old tmux, no server) must not quit the viewer.
                    let _ = selection::copy_to_tmux(&self.selection.text(rendered));
                    self.selection.clear();
                }
                // A click, not the end of a drag, on a link.
                let click = self.down.take() == Some((m.column, row));
                let url = link::url(rendered[(m.column, row)].symbol()).filter(|_| click);
                if let Some(url) = url.map(str::to_string) {
                    return self.go(Some(&url));
                }
            }
            _ => return Move::To(s),
        }
        self.repaint = true;
        Move::To(s)
    }
}
