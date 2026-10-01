//! Kitty graphics: uploading diagrams and drawing them through Unicode placeholders.

use crate::diacritics::DIACRITICS;
use base64::{Engine, engine::general_purpose::STANDARD};
use ratatui::{buffer::Buffer, style::Color};
use std::io::Write;

/// Kitty graphics escape, wrapped for tmux passthrough (`allow-passthrough on`) when needed.
pub fn kitty(out: &mut impl Write, body: &str, tmux: bool) -> std::io::Result<()> {
    let seq = format!("\x1b_G{body}\x1b\\");
    if tmux {
        write!(out, "\x1bPtmux;{}\x1b\\", seq.replace('\x1b', "\x1b\x1b"))
    } else {
        out.write_all(seq.as_bytes())
    }
}

/// Upload a PNG as a virtual placement (U=1) of `cols` x `rows` cells. Nothing is drawn
/// until placeholder cells with this id appear on screen; re-uploading an id replaces it.
pub fn upload(
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

/// Image `id`'s placeholder cells for image rows `rows`, the first drawn at buffer row `top`.
pub fn image_cells(buf: &mut Buffer, id: u32, cols: u16, rows: std::ops::Range<u16>, top: u16) {
    let fg = Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8);
    let first = rows.start;
    for r in rows {
        for c in 0..cols.min(buf.area.width) {
            buf[(c, top + r - first)]
                .set_symbol(&placeholder(r.into(), c.into()))
                .set_fg(fg);
        }
    }
}

/// Placeholder cell for image row `r`, column `c`. Its fg color carries the image id.
pub fn placeholder(r: usize, c: usize) -> String {
    format!("\u{10EEEE}{}{}", DIACRITICS[r], DIACRITICS[c])
}

/// Width and height from the PNG IHDR chunk.
pub fn png_size(png: &[u8]) -> (u32, u32) {
    let be = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
    (be(16), be(20))
}

/// Whether the terminal can show kitty images through Unicode placeholders: kitty and
/// Ghostty. Inside tmux, `TERM` only says tmux, so `tmux_client` is the outer terminal
/// as tmux reports it (`#{client_termname}`) and takes precedence over the environment.
/// ponytail: a heuristic, not a query; `--images` / `--no-images` override it.
pub fn supports_images(var: impl Fn(&str) -> Option<String>, tmux_client: Option<String>) -> bool {
    let graphics = |term: &str| term.contains("kitty") || term.contains("ghostty");
    if let Some(term) = tmux_client {
        return graphics(&term);
    }
    var("TERM").is_some_and(|t| graphics(&t))
        || var("TERM_PROGRAM").is_some_and(|p| p.eq_ignore_ascii_case("ghostty"))
        || var("KITTY_WINDOW_ID").is_some()
        || var("GHOSTTY_RESOURCES_DIR").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

    #[test]
    fn placeholder_is_one_cell_wide() {
        // ratatui lays cells out by display width; anything but 1 would shear the image.
        assert_eq!(Line::from(placeholder(3, 7)).width(), 1);
    }

    #[test]
    fn detects_terminals_with_kitty_image_placeholders() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        // (environment, tmux client terminal, expected)
        type Case = (
            &'static [(&'static str, &'static str)],
            Option<&'static str>,
            bool,
        );
        let cases: [Case; 8] = [
            (&[("TERM", "xterm-kitty")], None, true),
            (&[("TERM", "xterm-ghostty")], None, true),
            (
                &[("TERM", "xterm-256color"), ("TERM_PROGRAM", "ghostty")],
                None,
                true,
            ),
            (
                &[("TERM", "xterm-256color"), ("KITTY_WINDOW_ID", "1")],
                None,
                true,
            ),
            (&[("TERM", "xterm-256color")], None, false),
            (&[("TERM", "alacritty")], None, false),
            // Inside tmux, the client terminal as tmux reports it decides.
            (
                &[("TERM", "tmux-256color"), ("TERM_PROGRAM", "tmux")],
                Some("xterm-ghostty"),
                true,
            ),
            (
                &[("TERM", "tmux-256color"), ("GHOSTTY_RESOURCES_DIR", "/x")],
                Some("xterm-256color"),
                false,
            ),
        ];
        for (vars, client, expected) in cases {
            let got = supports_images(env(vars), client.map(str::to_string));
            assert_eq!(got, expected, "{vars:?} tmux client {client:?}");
        }
    }
}
