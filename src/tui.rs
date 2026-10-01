//! The interactive viewer: the event loop, keys and mouse.

use crate::kitty::refresh_tmux;
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

/// Where a key or mouse event moves the view.
enum Move {
    /// Scroll to this row, leaving the current search hit behind if it moves.
    To(u32),
    /// Scroll to this row and keep the current search hit: `n`, `N`, typing a query.
    Jump(u32),
    Quit,
}

struct Tui {
    v: Viewer,
    selection: Selection,
    /// Re-read and re-render the document: on start, resize, or a save.
    dirty: bool,
    /// The screen changed since the last tmux repaint.
    repaint: bool,
}

/// Take over the terminal and show `v` until the user quits.
pub fn run(v: Viewer, mouse: bool) -> Result<(), Box<dyn Error>> {
    let mut terminal = ratatui::init();
    let mut tui = Tui {
        v,
        selection: Selection::default(),
        dirty: true,
        repaint: false,
    };
    let result = tui.event_loop(&mut terminal, mouse);
    // Free the uploaded images, then hand the terminal back.
    tui.v.free_images(terminal.backend_mut())?;
    execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
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
                })?
                .buffer
                .clone();
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
                        self.key(k, page, max, size.width)
                    }
                    Event::Mouse(m) => Move::To(self.mouse(m, height, &rendered)),
                    Event::Resize(..) => {
                        self.dirty = true;
                        Move::To(s)
                    }
                    _ => Move::To(s),
                };
                let (row, jump) = match step {
                    Move::To(row) => (row, false),
                    Move::Jump(row) => (row, true),
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

    fn key(&mut self, k: KeyEvent, page: u32, max: u32, width: u16) -> Move {
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
            KeyCode::Char('n') => return Move::Jump(v.search.jump(s, false).unwrap_or(s)),
            KeyCode::Char('N') => return Move::Jump(v.search.jump(s, true).unwrap_or(s)),
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

    /// Wheel scrolling, and in tmux, selecting text to copy.
    fn mouse(&mut self, m: MouseEvent, height: u16, rendered: &Buffer) -> u32 {
        let s = self.v.scroll;
        // Keep selections off the search status line.
        let row = m.row.min(height.saturating_sub(1));
        let tmux = self.v.tmux;
        match m.kind {
            MouseEventKind::ScrollDown => {
                self.selection.clear();
                return s.saturating_add(3);
            }
            MouseEventKind::ScrollUp => {
                self.selection.clear();
                return s.saturating_sub(3);
            }
            MouseEventKind::Down(MouseButton::Left) if tmux => self.selection.start(m.column, row),
            MouseEventKind::Drag(MouseButton::Left) if tmux => self.selection.drag(m.column, row),
            MouseEventKind::Up(MouseButton::Left) if tmux => {
                self.selection.drag(m.column, row);
                // Best effort: a failed copy (old tmux, no server) must not quit the viewer.
                let _ = selection::copy_to_tmux(&self.selection.text(rendered));
                self.selection.clear();
            }
            _ => return s,
        }
        self.repaint = true;
        s
    }
}
