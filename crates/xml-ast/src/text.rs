//! Text ranges (byte offsets) and conversion between byte offsets and
//! line/column positions. Columns are counted in UTF-16 code units because
//! that is the default position encoding of the LSP.

use std::fmt;

/// A half-open byte range `[start, end)` in the source text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "invalid TextRange: {start} > {end}");
        Self { start, end }
    }

    pub fn empty(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether the range contains the offset (half-open, `end` excluded).
    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// Whether the range contains the offset including `end` — useful for
    /// cursor positions.
    pub fn contains_inclusive(&self, offset: usize) -> bool {
        self.start <= offset && offset <= self.end
    }

    /// The smallest range covering both ranges.
    pub fn cover(&self, other: TextRange) -> TextRange {
        TextRange::new(self.start.min(other.start), self.end.max(other.end))
    }
}

impl fmt::Debug for TextRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// Line and column, both zero-based; the column is in UTF-16 code units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// Index of a text's line starts for fast offset ↔ position conversion.
///
/// The index does not store the text itself; the conversion functions
/// expect the same text the index was built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    /// Byte offset of the start of each line; the first entry is always 0.
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            line_starts,
            len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte offset of the line start, if the line exists.
    pub fn line_start(&self, line: u32) -> Option<usize> {
        self.line_starts.get(line as usize).copied()
    }

    /// Converts a byte offset into line/column. Offsets outside the text or
    /// in the middle of a UTF-8 character are clamped downwards.
    pub fn line_col(&self, text: &str, offset: usize) -> LineCol {
        let mut offset = offset.min(self.len);
        while offset > 0 && !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let start = self.line_starts[line];
        let col: u32 = text[start..offset]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        LineCol {
            line: line as u32,
            col,
        }
    }

    /// Converts line/column into a byte offset. Positions past the end of a
    /// line are clamped to the line end, lines past the end of the text to
    /// the text end (as required by the LSP specification).
    pub fn offset(&self, text: &str, pos: LineCol) -> usize {
        let line = pos.line as usize;
        let Some(&start) = self.line_starts.get(line) else {
            return self.len;
        };
        // Line end without the trailing '\n'.
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(self.len, |&next| next.saturating_sub(1));
        let mut offset = start;
        let mut col = 0u32;
        for c in text[start..end].chars() {
            if col >= pos.col {
                break;
            }
            col += c.len_utf16() as u32;
            offset += c.len_utf8();
        }
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_ascii() {
        let text = "abc\ndef\n";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 3);
        assert_eq!(index.line_col(text, 0), LineCol { line: 0, col: 0 });
        assert_eq!(index.line_col(text, 3), LineCol { line: 0, col: 3 });
        assert_eq!(index.line_col(text, 4), LineCol { line: 1, col: 0 });
        assert_eq!(index.line_col(text, 8), LineCol { line: 2, col: 0 });
        assert_eq!(index.offset(text, LineCol { line: 1, col: 2 }), 6);
    }

    #[test]
    fn line_index_utf16_columns() {
        // 'ä' = 2 bytes / 1 UTF-16 unit, '😀' = 4 bytes / 2 UTF-16 units.
        let text = "ä😀x\ny";
        let index = LineIndex::new(text);
        assert_eq!(index.line_col(text, 2), LineCol { line: 0, col: 1 });
        assert_eq!(index.line_col(text, 6), LineCol { line: 0, col: 3 });
        assert_eq!(index.offset(text, LineCol { line: 0, col: 3 }), 6);
        assert_eq!(index.offset(text, LineCol { line: 1, col: 0 }), 8);
    }

    #[test]
    fn line_index_clamps_out_of_bounds() {
        let text = "ab\ncd";
        let index = LineIndex::new(text);
        // Columns past the line end are clamped to the line end.
        assert_eq!(index.offset(text, LineCol { line: 0, col: 99 }), 2);
        // Lines past the text end are clamped to the text end.
        assert_eq!(index.offset(text, LineCol { line: 9, col: 0 }), 5);
        assert_eq!(index.line_col(text, 999), LineCol { line: 1, col: 2 });
    }

    #[test]
    fn line_index_offset_in_char() {
        let text = "😀a";
        let index = LineIndex::new(text);
        // An offset in the middle of the emoji is clamped to its start.
        assert_eq!(index.line_col(text, 2), LineCol { line: 0, col: 0 });
    }

    #[test]
    fn text_range_basics() {
        let range = TextRange::new(2, 5);
        assert!(range.contains(2));
        assert!(range.contains(4));
        assert!(!range.contains(5));
        assert!(range.contains_inclusive(5));
        assert_eq!(range.cover(TextRange::new(4, 9)), TextRange::new(2, 9));
    }
}
