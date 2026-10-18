/// The layout of a rendered frame.
/// This is produced during rendering by [`crate::render::render_state`]
/// and then stored in [`crate::State`] such that layout dependent actions,
/// e.g. the "PageDown" keybinding, can refer to it.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    num_header_lines: u16,
    content_top_line: u16,
    content_bottom_line: u16,
}

/// How many blank lines to render between the header and the content.
const HEADER_GAP: u16 = 1;

impl Layout {
    pub fn new(term_size: [u16; 2], num_header_lines: u16) -> Self {
        Self {
            num_header_lines,
            content_top_line: num_header_lines + HEADER_GAP,
            content_bottom_line: term_size[1].saturating_sub(1),
        }
    }

    /// The height of the header.
    pub fn num_header_lines(&self) -> u16 {
        self.num_header_lines
    }

    /// The y-position of the first line of the content region.
    pub fn content_top_line(&self) -> u16 {
        self.content_top_line
    }

    /// The y-position of the last line of the content region.
    pub fn content_bottom_line(&self) -> u16 {
        self.content_bottom_line
    }

    /// The y-position of the status line.
    pub fn status_line(&self) -> u16 {
        self.content_bottom_line
    }

    /// The number of lines in the content region.
    pub fn num_content_lines(&self) -> u16 {
        self.content_bottom_line
            .saturating_sub(self.content_top_line)
            .saturating_add(1)
    }

    /// The number of lines used by the file table for entries.
    /// This number does not include the table's header row.
    pub fn num_file_list_lines(&self) -> u16 {
        self.num_content_lines().saturating_sub(1)
    }
}
