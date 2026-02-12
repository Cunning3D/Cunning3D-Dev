//! DisplayMap: buffer↔display row mapping (wrap/fold-ready).

#[derive(Debug, Clone, Copy)]
pub struct DisplayRow {
    pub buffer_line: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub is_continuation: bool,
}

#[derive(Debug, Clone)]
pub struct DisplayMap {
    wrap_cols: Option<usize>,
    line_row_range: Vec<(usize, usize)>, // start_row, row_count
    rows: Vec<DisplayRow>,
}

impl DisplayMap {
    pub fn new(text_lines: &[String], wrap_cols: Option<usize>) -> Self {
        let mut rows = Vec::new();
        let mut line_row_range = Vec::with_capacity(text_lines.len());
        for (li, line) in text_lines.iter().enumerate() {
            let start_row = rows.len();
            let breaks = Self::wrap_breaks(line, wrap_cols);
            let mut seg = 0usize;
            for w in breaks.windows(2) {
                rows.push(DisplayRow {
                    buffer_line: li,
                    byte_start: w[0],
                    byte_end: w[1],
                    is_continuation: seg > 0,
                });
                seg += 1;
            }
            if breaks.len() < 2 {
                rows.push(DisplayRow { buffer_line: li, byte_start: 0, byte_end: line.len(), is_continuation: false });
            }
            line_row_range.push((start_row, rows.len() - start_row));
        }
        if rows.is_empty() {
            rows.push(DisplayRow { buffer_line: 0, byte_start: 0, byte_end: 0, is_continuation: false });
            line_row_range.push((0, 1));
        }
        Self { wrap_cols, line_row_range, rows }
    }

    pub fn wrap_cols(&self) -> Option<usize> { self.wrap_cols }
    pub fn row_count(&self) -> usize { self.rows.len().max(1) }
    pub fn row(&self, display_row: usize) -> DisplayRow { self.rows.get(display_row).copied().unwrap_or(DisplayRow { buffer_line: 0, byte_start: 0, byte_end: 0, is_continuation: false }) }
    pub fn line_first_row(&self, line: usize) -> usize { self.line_row_range.get(line).map(|x| x.0).unwrap_or(0) }

    pub fn buffer_to_display_row(&self, line: usize, col_byte: usize) -> usize {
        let (start, n) = self.line_row_range.get(line).copied().unwrap_or((0, 1));
        let col = col_byte;
        for i in 0..n {
            let r = self.rows.get(start + i).copied().unwrap();
            if col >= r.byte_start && col <= r.byte_end { return start + i; }
        }
        start
    }

    pub fn display_to_buffer(&self, display_row: usize) -> (usize, usize) {
        let r = self.row(display_row);
        (r.buffer_line, r.byte_start)
    }

    fn wrap_breaks(line: &str, wrap_cols: Option<usize>) -> Vec<usize> {
        let Some(cols) = wrap_cols.filter(|c| *c > 0) else { return vec![0, line.len()]; };
        let mut out = vec![0usize];
        let mut count = 0usize;
        for (i, ch) in line.char_indices() {
            if count >= cols {
                out.push(i);
                count = 0;
            }
            count += if ch == '\t' { 4 } else { 1 };
        }
        if *out.last().unwrap_or(&0) != line.len() { out.push(line.len()); }
        if out.len() == 1 { vec![0, line.len()] } else { out }
    }
}

