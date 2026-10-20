use std::{fmt::Display, io::Write};

use crossterm::style::{
    Attribute::{Bold, Italic, Reverse, Underlined},
    ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use serde::Deserialize;
use serde_default_utils::serde_inline_default;

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct Theme {
    #[serde_inline_default(Header::default())]
    pub header: Header,
    #[serde_inline_default(FileTable::default())]
    pub file_table: FileTable,
    #[serde_inline_default(ErrorView::default())]
    pub error_view: ErrorView,
    #[serde_inline_default(HelpView::default())]
    pub help_view: HelpView,
    #[serde_inline_default(DeleteDialog::default())]
    pub delete_dialog: DeleteDialog,
    #[serde_inline_default([0.3, 0.3, 0.3])]
    pub size_min_color: Color,
    #[serde_inline_default([1.0, 0.3, 0.3])]
    pub size_max_color: Color,
}

const C1: [f32; 3] = [0.4, 0.5, 1.0];
const C2: [f32; 3] = [0.75, 0.5, 1.0];
const C3: [f32; 3] = [1.0, 0.5, 0.5];

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct Header {
    #[serde_inline_default(Style::default())]
    pub label: Style,

    #[serde_inline_default(Style::default().fg(C1))]
    pub path_prefix: Style,

    #[serde_inline_default(Style::default().fg(C2))]
    pub path_suffix: Style,

    #[serde_inline_default(Style::default().fg(C2))]
    pub size_current: Style,

    #[serde_inline_default(Style::default().fg(C1))]
    pub size_total: Style,

    /// Defaults to using a color between [`size_min_color`] and [`size_max_color`] depending on percentage.
    #[serde_inline_default(None)]
    pub size_percent: Option<Style>,

    #[serde_inline_default(Style::default().fg(C1))]
    pub sort_mode: Style,

    /// Style of the size metric value (e.g. `freed`) displayed after the
    /// size.
    #[serde_inline_default(Style::default().fg(C1))]
    pub size_metric: Style,

    #[serde_inline_default(Style::default().fg(C3))]
    pub error: Style,

    #[serde_inline_default(Style::default())]
    pub spinner: Style,
}

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct FileTable {
    #[serde_inline_default(Style::default().bold())]
    pub progress_header: Style,

    #[serde_inline_default(Style::default().bold())]
    pub name_header: Style,

    #[serde_inline_default(Style::default().bold())]
    pub size_abs_header: Style,

    #[serde_inline_default(Style::default().bold())]
    pub size_rel_header: Style,

    #[serde_inline_default(Style::default().bold())]
    pub size_cur_header: Style,

    #[serde_inline_default(Style::default().bold())]
    pub modtime_header: Style,

    #[serde_inline_default(Style::default())]
    pub progress_value: Style,

    #[serde_inline_default(Style::default())]
    pub name_file_value: Style,

    #[serde_inline_default(Style::default().fg([0.7, 0.7, 0.7]))]
    pub name_dir_value: Style,

    #[serde_inline_default(Style::default().fg([0.9, 0.9, 0.6]))]
    pub name_hardlink_value: Style,

    #[serde_inline_default(Style::default())]
    pub size_abs_value: Style,

    /// Defaults to using a color between [`size_min_color`] and [`size_max_color`] depending on percentage.
    #[serde_inline_default(None)]
    pub size_rel_value: Option<Style>,

    /// Defaults to using a color between [`size_min_color`] and [`size_max_color`] depending on percentage.
    #[serde_inline_default(None)]
    pub size_cur_value: Option<Style>,

    #[serde_inline_default(Style::default())]
    pub modtime_value: Style,

    #[serde_inline_default(Style::default().fg([0.5, 0.5, 0.5]))]
    pub modtime_delim: Style,

    #[serde_inline_default(Style::default().bg([0.2, 0.2, 0.2]))]
    pub selected_row: Style,

    #[serde_inline_default(Style::default().fg(C3))]
    pub error_row: Style,

    #[serde_inline_default(Style::default().fg(C3).bg([0.2, 0.2, 0.2]))]
    pub selected_error_row: Style,
}

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct ErrorView {
    #[serde_inline_default(Style::default())]
    pub index: Style,
    #[serde_inline_default(Style::default().fg([1.0, 0.5, 0.5]).bold())]
    pub path: Style,
    #[serde_inline_default(Style::default().fg([0.8, 0.3, 0.3]))]
    pub message: Style,
}

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct HelpView {
    #[serde_inline_default(Style::default().fg([0.8, 0.5, 0.8]).bold())]
    pub keys_header: Style,
    #[serde_inline_default(Style::default().fg([0.8, 0.8, 0.5]).bold())]
    pub action_header: Style,
    #[serde_inline_default(Style::default().fg([0.5, 0.8, 0.8]).bold())]
    pub scope_header: Style,

    #[serde_inline_default(Style::default().fg([0.8, 0.5, 0.8]))]
    pub keys: Style,
    #[serde_inline_default(Style::default().fg([0.8, 0.8, 0.5]))]
    pub action: Style,
    #[serde_inline_default(Style::default().fg([0.5, 0.8, 0.8]))]
    pub scope: Style,
}

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct DeleteDialog {
    #[serde_inline_default(Style::default().fg([0.4, 0.5, 1.0]))]
    pub path: Style,
    #[serde_inline_default(Style::default())]
    pub message: Style,
}

impl Default for Theme {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl Default for Header {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl Default for FileTable {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl Default for ErrorView {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl Default for HelpView {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

impl Default for DeleteDialog {
    fn default() -> Self {
        toml::from_str("").unwrap()
    }
}

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize, Default)]
pub struct Style {
    #[serde_inline_default(None)]
    pub fg: Option<Color>,

    #[serde_inline_default(None)]
    pub bg: Option<Color>,

    #[serde_inline_default(false)]
    pub bold: bool,

    #[serde_inline_default(false)]
    pub italic: bool,

    #[serde_inline_default(false)]
    pub underlined: bool,

    #[serde_inline_default(false)]
    pub reversed: bool,
}

impl Style {
    pub fn fg(mut self, v: Color) -> Self {
        self.fg = Some(v);
        self
    }
    pub fn bg(mut self, v: Color) -> Self {
        self.bg = Some(v);
        self
    }
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
    pub fn underlined(mut self) -> Self {
        self.underlined = true;
        self
    }
    pub fn reversed(mut self) -> Self {
        self.reversed = true;
        self
    }
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            fg: other.fg.or(self.fg),
            bg: other.bg.or(self.bg),
            bold: other.bold,
            italic: other.italic,
            underlined: other.underlined,
            reversed: other.reversed,
        }
    }
    pub fn write(&self, w: &mut impl Write) -> Result<(), std::io::Error> {
        if let Some(fg) = self.fg {
            write!(w, "{}", SetForegroundColor(color_to_crossterm(&fg)))?;
        }
        if let Some(bg) = self.bg {
            write!(w, "{}", SetBackgroundColor(color_to_crossterm(&bg)))?;
        }
        if self.bold {
            write!(w, "{}", SetAttribute(Bold))?;
        }
        if self.italic {
            write!(w, "{}", SetAttribute(Italic))?;
        }
        if self.underlined {
            write!(w, "{}", SetAttribute(Underlined))?;
        }
        if self.reversed {
            write!(w, "{}", SetAttribute(Reverse))?;
        }
        Ok(())
    }
    pub fn write_text(&self, w: &mut impl Write, content: &str) -> Result<(), std::io::Error> {
        self.write(w)?;
        write!(w, "{}{}", content, ResetColor)?;
        Ok(())
    }
    pub fn write_spaces(&self, w: &mut impl Write, n: usize) -> Result<(), std::io::Error> {
        self.write_text(w, &(0..n).map(|_| ' ').collect::<String>())
    }
}

impl Display for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut buf = Vec::new();
        self.write(&mut buf).map_err(|_| std::fmt::Error)?;
        let s = std::str::from_utf8(&buf).map_err(|_| std::fmt::Error)?;
        f.write_str(s)
    }
}

pub type Color = [f32; 3];

pub fn color_to_crossterm(c: &Color) -> crossterm::style::Color {
    crossterm::style::Color::Rgb {
        r: (c[0] * 255.0) as u8,
        g: (c[1] * 255.0) as u8,
        b: (c[2] * 255.0) as u8,
    }
}

pub fn interpolate_color(f: f32, c1: Color, c2: Color) -> Color {
    [
        c1[0] * (1.0 - f) + c2[0] * f,
        c1[1] * (1.0 - f) + c2[1] * f,
        c1[2] * (1.0 - f) + c2[2] * f,
    ]
}
