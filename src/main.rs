mod cli;
mod diacritics;
mod kitty;
mod search;
mod selection;
mod viewer;

use cli::parse_args;
use kitty::{kitty, supports_images};
use ratatui::crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
};
use std::{
    io::{IsTerminal, Read},
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
use viewer::Viewer;
use yamdview::Theme;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tmux = std::env::var_os("TMUX").is_some();
    let args = parse_args(std::env::args_os().skip(1))?;
    // `-`, or a pipe with no file given: read stdin now, before the TUI takes over.
    // Keys still work: crossterm reads them from /dev/tty when stdin isn't a terminal.
    let stdin = if args.read_stdin || (args.path.is_none() && !std::io::stdin().is_terminal()) {
        let mut md = String::new();
        std::io::stdin().read_to_string(&mut md)?;
        Some(md)
    } else {
        None
    };
    let path = args
        .path
        .unwrap_or_else(|| PathBuf::from(if stdin.is_some() { "-" } else { "README.md" }));
    let images = args.images.unwrap_or_else(|| {
        let tmux_client = tmux
            .then(|| {
                Command::new("tmux")
                    .args(["display-message", "-p", "#{client_termname}"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|t| !t.is_empty())
            })
            .flatten();
        supports_images(|k| std::env::var(k).ok(), tmux_client)
    });
    let mut v = Viewer::new(path, stdin, tmux, images, Theme::detect());
    // Print instead of opening the viewer: styled with --print (for the terminal, and so
    // tmux's history), plain whenever stdout isn't a terminal (a pipe or a file).
    let terminal_out = std::io::stdout().is_terminal();
    if args.print || !terminal_out {
        v.images &= args.print && terminal_out;
        let width = ratatui::crossterm::terminal::size().map_or(80, |(w, _)| w);
        match v.print(&mut std::io::stdout().lock(), width, args.print) {
            // The reader stopped early (`| head`): that's not an error.
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            result => result?,
        }
        if v.tmux && v.images {
            // Same Ghostty-in-tmux repaint as the viewer's; best effort, output discarded.
            let _ = Command::new("tmux").arg("refresh-client").output();
        }
        return Ok(());
    }
    let mut terminal = ratatui::init();

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        if args.mouse {
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
                    // Best effort and silent: with no client attached (a detached
                    // session), tmux's error would land on our screen and shift it.
                    let _ = Command::new("tmux")
                        .arg("refresh-client")
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
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
                            KeyCode::Char(']') => v.heading(s, false).unwrap_or(s),
                            KeyCode::Char('[') => v.heading(s, true).unwrap_or(s),
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
