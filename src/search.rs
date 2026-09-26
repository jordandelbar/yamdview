use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Widget},
};

#[derive(Debug, PartialEq)]
pub struct Hit {
    pub row: u16,
    pub start: u16,
    pub end: u16,
}

#[derive(Default)]
pub struct Search {
    pub query: String,
    pub editing: bool,
    pub hits: Vec<Hit>,
    pub current: Option<usize>,
}

impl Search {
    pub fn index(&mut self, paragraph: &Paragraph<'_>, width: u16, height: u16, offset: u16) {
        if self.query.is_empty() || width == 0 {
            return;
        }
        // Use the viewer's wrapping and cell widths, with bounded temporary storage.
        for first_row in (0..height).step_by(128) {
            let area = Rect::new(0, 0, width, (height - first_row).min(128));
            let mut buffer = Buffer::empty(area);
            paragraph
                .clone()
                .scroll((first_row, 0))
                .render(area, &mut buffer);
            for row in 0..area.height {
                let mut text = String::new();
                let mut cells = Vec::new();
                let mut covered = 0;
                for x in 0..width {
                    if x < covered {
                        continue;
                    }
                    let symbol = buffer[(x, row)].symbol();
                    let cell_width = ratatui::text::Span::raw(symbol).width() as u16;
                    covered = x + cell_width.max(1);
                    let start = text.len();
                    text.push_str(symbol);
                    cells.push((start, text.len(), x, covered));
                }
                for (start, matched) in text.match_indices(&self.query) {
                    let end = start + matched.len();
                    let first = cells.iter().find(|c| c.1 > start).unwrap();
                    let last = cells.iter().rev().find(|c| c.0 < end).unwrap();
                    self.hits.push(Hit {
                        row: offset.saturating_add(first_row).saturating_add(row),
                        start: first.2,
                        end: last.3,
                    });
                }
            }
        }
    }

    pub fn jump(&mut self, scroll: u16, backwards: bool) -> Option<u16> {
        let count = self.hits.len();
        if count == 0 {
            self.current = None;
            return None;
        }
        let index = match self.current {
            Some(i) if backwards => (i + count - 1) % count,
            Some(i) => (i + 1) % count,
            None if backwards => self
                .hits
                .iter()
                .rposition(|h| h.row <= scroll)
                .unwrap_or(count - 1),
            None => self.hits.iter().position(|h| h.row >= scroll).unwrap_or(0),
        };
        self.current = Some(index);
        Some(self.hits[index].row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        text::{Line, Span},
        widgets::Wrap,
    };

    #[test]
    fn finds_styled_unicode_and_wrapped_text_at_document_rows() {
        let p = Paragraph::new(Line::from(vec![Span::raw("猫 "), Span::raw("hello hello")]))
            .wrap(Wrap { trim: false });
        let mut search = Search {
            query: "hello".into(),
            ..Search::default()
        };
        search.index(&p, 9, p.line_count(9) as u16, 4);
        assert_eq!(
            search.hits,
            vec![
                Hit {
                    row: 4,
                    start: 3,
                    end: 8
                },
                Hit {
                    row: 5,
                    start: 0,
                    end: 5
                }
            ]
        );
        assert_eq!(search.jump(0, false), Some(4));
        assert_eq!(search.jump(4, false), Some(5));
        assert_eq!(search.jump(5, false), Some(4));
        assert_eq!(search.jump(4, true), Some(5));
    }

    #[test]
    fn empty_and_missing_queries_have_no_target() {
        let mut search = Search::default();
        search.index(&Paragraph::new("hello"), 10, 1, 0);
        assert_eq!(search.jump(0, false), None);
        search.query = "missing".into();
        search.index(&Paragraph::new("hello"), 10, 1, 0);
        assert_eq!(search.jump(0, true), None);
    }
}
