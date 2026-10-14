use std::io::Write;

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
    #[serde_inline_default([0.2, 0.2, 0.2])]
    pub size_min_color: Color,
    #[serde_inline_default([1.0, 0.2, 0.2])]
    pub size_max_color: Color,
}

#[serde_inline_default]
#[derive(Clone, Debug, Deserialize)]
pub struct Header {
    #[serde_inline_default(Style::default())]
    pub label: Style,

    #[serde_inline_default(Style::default().fg([0.4, 0.5, 1.0]))]
    pub path_prefix: Style,

    #[serde_inline_default(Style::default().fg([1.0, 0.4, 0.5]))]
    pub path_suffix: Style,

    #[serde_inline_default(Style::default().fg([1.0, 0.4, 0.5]))]
    pub size_current: Style,

    #[serde_inline_default(Style::default().fg([0.4, 0.5, 1.0]))]
    pub size_total: Style,

    /// Defaults to using a color between {`size_min_color`] and [`size_max_color`] depending on percentage.
    #[serde_inline_default(None)]
    pub size_percent: Option<Style>,

    #[serde_inline_default(Style::default().fg([0.4, 0.5, 1.0]))]
    pub sort_mode: Style,
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

    #[serde_inline_default(Style::default())]
    pub size_abs_value: Style,

    /// Defaults to using a color between {`size_min_color`] and [`size_max_color`] depending on percentage.
    #[serde_inline_default(None)]
    pub size_rel_value: Option<Style>,

    /// Defaults to using a color between {`size_min_color`] and [`size_max_color`] depending on percentage.
    #[serde_inline_default(None)]
    pub size_cur_value: Option<Style>,

    #[serde_inline_default(Style::default())]
    pub modtime_value: Style,

    #[serde_inline_default(Style::default().fg([0.5, 0.5, 0.5]))]
    pub modtime_delim: Style,

    #[serde_inline_default(Style::default().bg([0.1, 0.1, 0.1]))]
    pub selected_row: Style,
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

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
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
            fg: self.fg.clone().or_else(|| other.fg.clone()),
            bg: self.bg.clone().or_else(|| other.bg.clone()),
            bold: other.bold,
            italic: other.italic,
            underlined: other.underlined,
            reversed: other.reversed,
        }
    }
    pub fn write(&self, mut w: impl Write) -> Result<(), std::io::Error> {
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
    pub fn write_text(&self, mut w: impl Write, content: &str) -> Result<(), std::io::Error> {
        self.write(&mut w)?;
        write!(w, "{}{}", content, ResetColor)?;
        Ok(())
    }
    pub fn to_string(&self) -> String {
        let mut buf = Vec::new();
        self.write(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
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
