use ratatui::{
    buffer::{Buffer, Cell},
    layout::Rect,
    widgets::{Paragraph, Widget},
};

#[derive(Debug, PartialEq)]
pub struct Hit {
    pub row: u32,
    pub start: u16,
    pub end: u16,
}

struct RenderedRow {
    row: u32,
    text: String,
    // Byte offset and terminal cell range of each rendered grapheme.
    cells: Vec<(usize, u16, u16)>,
}

#[derive(Default)]
pub struct Search {
    pub query: String,
    pub editing: bool,
    pub hits: Vec<Hit>,
    pub current: Option<usize>,
    pub origin: u32,
    // None until a search needs it, so plain viewing never pays for the layout.
    rows: Option<Vec<RenderedRow>>,
}

impl Search {
    pub fn cached(&self) -> bool {
        self.rows.is_some()
    }

    pub fn cache(&mut self, paragraph: &Paragraph<'_>, width: u16, height: u16, offset: u32) {
        let rows = self.rows.get_or_insert_default();
        if width == 0 {
            return;
        }
        // Cache layout once per search and rebuild, not on every search keystroke.
        // Paragraph rewraps from the start for each 128-row chunk, which stays cheap
        // because `paragraphs` keeps blocks near 1024 rows; cell storage is bounded.
        for first_row in (0..height).step_by(128) {
            let area = Rect::new(0, 0, width, (height - first_row).min(128));
            let mut empty = Cell::default();
            empty.set_symbol("");
            let mut buffer = Buffer::filled(area, empty);
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
                    if symbol.is_empty() {
                        continue;
                    }
                    let cell_width = ratatui::text::Span::raw(symbol).width() as u16;
                    covered = x + cell_width.max(1);
                    let start = text.len();
                    text.push_str(symbol);
                    cells.push((start, x, covered));
                }
                rows.push(RenderedRow {
                    row: offset + u32::from(first_row + row),
                    text,
                    cells,
                });
            }
        }
    }

    pub fn clear_layout(&mut self) {
        self.rows = None;
        self.hits.clear();
        self.current = None;
    }

    pub fn refresh(&mut self) {
        self.hits.clear();
        self.current = None;
        if self.query.is_empty() {
            return;
        }
        for row in self.rows.iter().flatten() {
            for (start, matched) in row.text.match_indices(&self.query) {
                let end = start + matched.len();
                let first = row.cells.partition_point(|c| c.0 <= start) - 1;
                let last = row.cells.partition_point(|c| c.0 < end) - 1;
                self.hits.push(Hit {
                    row: row.row,
                    start: row.cells[first].1,
                    end: row.cells[last].2,
                });
            }
        }
    }

    pub fn begin(&mut self, scroll: u32) {
        self.origin = scroll;
        self.editing = true;
        self.query.clear();
        self.refresh();
    }

    pub fn cancel(&mut self) -> u32 {
        self.editing = false;
        self.query.clear();
        self.clear_layout();
        self.origin
    }

    pub fn incremental(&mut self) -> u32 {
        self.refresh();
        self.current = self
            .hits
            .iter()
            .position(|h| h.row >= self.origin)
            .or_else(|| (!self.hits.is_empty()).then_some(0));
        self.current.map_or(self.origin, |i| self.hits[i].row)
    }

    pub fn jump(&mut self, scroll: u32, backwards: bool) -> Option<u32> {
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
                .rposition(|h| h.row < scroll)
                .unwrap_or(count - 1),
            None => self.hits.iter().position(|h| h.row > scroll).unwrap_or(0),
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
        search.cache(&p, 9, p.line_count(9) as u16, 4);
        search.refresh();
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
        search.cache(&Paragraph::new("hello"), 10, 1, 0);
        search.refresh();
        assert_eq!(search.jump(0, false), None);
        search.query = "missing".into();
        search.cache(&Paragraph::new("hello"), 10, 1, 0);
        search.refresh();
        assert_eq!(search.jump(0, true), None);
    }

    #[test]
    fn navigation_after_scrolling_or_rebuild_skips_the_current_row() {
        let mut search = Search::default();
        search.cache(
            &Paragraph::new("hit hit\nother\nhit\nother\nhit"),
            20,
            5,
            10,
        );
        search.begin(10);
        search.query = "hit".into();
        assert_eq!(search.incremental(), 10);
        assert_eq!(search.jump(10, false), Some(10)); // second hit on the same row
        assert_eq!(search.current, Some(1));
        search.current = None; // manual scrolling invalidates the previous target
        assert_eq!(search.jump(12, false), Some(14));
        search.refresh(); // reload/resize also invalidates the target
        assert_eq!(search.jump(12, false), Some(14));
        search.refresh();
        assert_eq!(search.jump(12, true), Some(10));
        search.refresh();
        assert_eq!(search.jump(14, false), Some(10));
        search.refresh();
        assert_eq!(search.jump(10, true), Some(14));
    }

    #[test]
    fn incremental_search_uses_original_position_and_cancel_restores_it() {
        let mut search = Search::default();
        search.cache(&Paragraph::new("alpha\nbeta\nalpha"), 10, 3, 10);
        search.begin(11);
        search.query = "alpha".into();
        assert_eq!(search.incremental(), 12);
        search.query = "beta".into();
        assert_eq!(search.incremental(), 11);
        search.query = "missing".into();
        assert_eq!(search.incremental(), 11);
        assert_eq!(search.cancel(), 11);
        assert!(!search.editing);
        assert!(search.query.is_empty());
        assert!(search.hits.is_empty());
    }

    #[test]
    fn cache_excludes_padding_but_keeps_real_spaces_and_wide_characters() {
        let mut search = Search::default();
        search.cache(&Paragraph::new("猫 end\nend next"), 20, 2, 0);
        search.query = "end ".into();
        search.refresh();
        assert_eq!(
            search.hits,
            vec![Hit {
                row: 1,
                start: 0,
                end: 4
            }]
        );
        search.query = "猫 ".into();
        search.refresh();
        assert_eq!(
            search.hits,
            vec![Hit {
                row: 0,
                start: 0,
                end: 3
            }]
        );
        search.clear_layout();
        search.cache(&Paragraph::new("new text"), 20, 1, 0);
        search.refresh();
        assert!(search.hits.is_empty());
    }

    #[test]
    fn cache_keeps_rows_across_chunk_boundaries() {
        let mut search = Search::default();
        let text = (0..260)
            .map(|i| format!("row{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        search.cache(&Paragraph::new(text), 10, 260, 3);
        search.query = "row128".into();
        search.refresh();
        assert_eq!(
            search.hits,
            vec![Hit {
                row: 131,
                start: 0,
                end: 6
            }]
        );
    }
}
