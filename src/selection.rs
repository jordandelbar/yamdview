use ratatui::{buffer::Buffer, layout::Position, style::Color};
use std::{
    io::{self, Write},
    process::{Command, Stdio},
};

#[derive(Default)]
pub struct Selection {
    anchor: Option<Position>,
    end: Option<Position>,
}

impl Selection {
    pub fn start(&mut self, x: u16, y: u16) {
        self.anchor = Some(Position::new(x, y));
        self.end = self.anchor;
    }

    pub fn drag(&mut self, x: u16, y: u16) {
        if self.anchor.is_some() {
            self.end = Some(Position::new(x, y));
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn contains(&self, x: u16, y: u16) -> bool {
        let (Some(a), Some(b)) = (self.anchor, self.end) else {
            return false;
        };
        let (a, b) = ((a.y, a.x), (b.y, b.x));
        (a.min(b)..=a.max(b)).contains(&(y, x))
    }

    pub fn highlight(&self, buffer: &mut Buffer, color: Color) {
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if self.contains(x, y) {
                    buffer[(x, y)].set_bg(color);
                }
            }
        }
    }

    pub fn text(&self, buffer: &Buffer) -> String {
        if self.anchor == self.end {
            return String::new();
        }
        let mut lines = Vec::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            let mut selected = false;
            let mut covered = 0;
            for x in 0..buffer.area.width {
                if x < covered {
                    continue;
                }
                let symbol = buffer[(x, y)].symbol();
                let width = ratatui::text::Span::raw(symbol).width().max(1) as u16;
                covered = x.saturating_add(width);
                if (x..covered).any(|col| self.contains(col, y)) {
                    selected = true;
                    // Kitty placeholders carry image metadata, not selectable text.
                    if !symbol.starts_with('\u{10EEEE}') {
                        line.push_str(symbol);
                    }
                }
            }
            if selected {
                lines.push(line.trim_end().to_string());
            }
        }
        lines.join("\n")
    }
}

pub fn copy_to_tmux(text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let mut child = Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let write = child.stdin.take().unwrap().write_all(text.as_bytes());
    let output = child.wait_with_output()?;
    write?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        layout::Rect,
        widgets::{Paragraph, Widget},
    };

    #[test]
    fn copies_reverse_multiline_unicode_without_padding_or_image_metadata() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);
        Paragraph::new("a猫b\n\u{10EEEE}diagram\nlast line").render(area, &mut buffer);
        let mut selection = Selection::default();
        selection.start(3, 2);
        selection.drag(1, 0);
        assert_eq!(selection.text(&buffer), "猫b\ndiagram\nlast");
        selection.clear();
        assert_eq!(selection.text(&buffer), "");
        selection.start(0, 0);
        assert_eq!(selection.text(&buffer), "");
    }
}
