use std::io::Write;

use crossterm::{
    cursor,
    terminal::{Clear, ClearType},
};

use crate::theme::Style;

/// The buffered terminal output of a single frame.
pub struct Frame {
    buf: Vec<u8>,
}

impl Frame {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn clear(&mut self) {
        let _ = write!(self.buf, "{}", Clear(ClearType::All));
    }

    pub fn line(&mut self, y: u16) -> LineWriter<'_> {
        LineWriter::start(&mut self.buf, y)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

/// Writes a single line. Tracks the cursor column, so content can be placed by
/// advancing or padding instead of computing absolute positions.
pub struct LineWriter<'a> {
    buf: &'a mut Vec<u8>,
    x: u16,
    y: u16,
}

impl<'a> LineWriter<'a> {
    fn start(buf: &'a mut Vec<u8>, y: u16) -> Self {
        let mut w = Self { buf, x: 0, y };
        w.move_to(0);
        w
    }

    fn move_to(&mut self, x: u16) {
        self.x = x;
        let _ = write!(self.buf, "{}", cursor::MoveTo(x, self.y));
    }

    /// Write styled text at the cursor position, advancing the cursor.
    pub fn text(&mut self, style: &Style, content: &str) {
        let _ = style.write_text(self.buf, content);
        self.x = self.x.saturating_add(content.chars().count() as u16);
    }

    /// Write `n` styled spaces, advancing the cursor.
    pub fn spaces(&mut self, style: &Style, n: usize) {
        let _ = style.write_spaces(self.buf, n);
        self.x = self.x.saturating_add(n as u16);
    }

    /// Write styled spaces until the cursor reaches column `x`.
    pub fn spaces_until(&mut self, style: &Style, x: u16) {
        let n = x.saturating_sub(self.x) as usize;
        self.spaces(style, n);
    }

    /// Fill columns `0..width` with a styled background, leaving the cursor
    /// at the start of the row.
    pub fn background(&mut self, style: &Style, width: usize) {
        self.spaces(style, width);
        self.move_to(0);
    }

    /// Move the cursor to the start of the next row.
    pub fn next_line(&mut self) {
        self.y += 1;
        self.move_to(0);
    }

    pub fn y(&self) -> u16 {
        self.y
    }
}
