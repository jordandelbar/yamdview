mod cli;
mod diacritics;
mod kitty;
mod search;
mod selection;
mod tui;
mod viewer;

use std::{
    error::Error,
    io::{IsTerminal, Read},
    path::PathBuf,
};
use viewer::Viewer;
use yamdview::Theme;

fn main() -> Result<(), Box<dyn Error>> {
    let args = cli::parse_args(std::env::args_os().skip(1))?;
    let tmux = std::env::var_os("TMUX").is_some();
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
    let images = args.images.unwrap_or_else(|| kitty::detect_images(tmux));
    let mut v = Viewer::new(path, stdin, tmux, images, Theme::detect());
    // Print instead of opening the viewer: styled with --print (for the terminal, and so
    // tmux's history), plain whenever stdout isn't a terminal (a pipe or a file).
    let terminal_out = std::io::stdout().is_terminal();
    if args.print || !terminal_out {
        v.images &= args.print && terminal_out;
        return print(v, args.print);
    }
    tui::run(v, args.mouse)
}

fn print(mut v: Viewer, styled: bool) -> Result<(), Box<dyn Error>> {
    let width = ratatui::crossterm::terminal::size().map_or(80, |(w, _)| w);
    match v.print(&mut std::io::stdout().lock(), width, styled) {
        // The reader stopped early (`| head`): that's not an error.
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
        result => result?,
    }
    if v.tmux && v.images {
        // Same Ghostty-in-tmux repaint as the viewer's.
        kitty::refresh_tmux();
    }
    Ok(())
}
