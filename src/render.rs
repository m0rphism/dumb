use std::{
    io::{Stdout, Write},
    path::Path,
};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::style::Color;

use crate::{
    DisplayMode, ErrorView, HelpView, State,
    config::Config,
    file_tree::FileState,
    frame::Frame,
    help::HELP,
    layout::Layout,
    snapshot::Snapshot,
    theme::{Style, Theme, interpolate_color},
};

pub fn render_state(
    stdout: &mut Stdout,
    state: &State,
    snapshot: &Snapshot,
    theme: &Theme,
    config: &Config,
) -> Result<Layout> {
    // We buffer all rendering before sending it to stdout to remove terminal flickering.
    // Flickering shouldn't be an issue if stdout is only flushed at the end, but for some reason
    // it is...
    let mut frame = Frame::new();

    frame.clear();

    let header_height = render_header(&mut frame, state, snapshot, theme, config);
    let layout = Layout::new(state.term_size, header_height);
    match &state.display_mode {
        DisplayMode::FileTree => {
            render_file_tree(&mut frame, state, snapshot, theme, &layout);
        }
        DisplayMode::ErrorView(error_view) => {
            render_errors(&mut frame, state, theme, error_view, &layout);
        }
        DisplayMode::HelpView(help_view) => {
            render_help_view(&mut frame, state, theme, help_view, &layout);
        }
        DisplayMode::DeleteDialog(target) => {
            render_delete_dialog(&mut frame, theme, &layout, &state.root_dir.join(target));
        }
    };

    // Status message (e.g. a failed deletion) on the bottom line.
    if let Some(msg) = &state.status {
        let width = state.term_size[0].saturating_sub(1) as usize;
        let msg: String = msg.chars().take(width).collect();
        let mut line = frame.line(layout.status_line());
        line.text(&theme.header.error, &msg);
    }

    stdout.write_all(frame.bytes())?;
    stdout.flush()?;

    Ok(layout)
}

/// The number of spaces between two adjacent columns of the file table.
const COLUMN_GAP: u16 = 2;

/// A column of the file table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// Entry name. The only column whose width is not fixed.
    Name,
    /// Absolute size of the entry.
    SizeAbs,
    /// Size of the entry relative to the total size of the root directory.
    SizeRel,
    /// Size of the entry relative to the current directory.
    SizeCur,
    /// Last modification time of the entry.
    ModTime,
}

impl Column {
    pub fn header(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::SizeAbs => "Size Abs",
            Column::SizeRel => "Size Rel",
            Column::SizeCur => "Size Cur",
            Column::ModTime => "Last Modified",
        }
    }

    pub fn width(self, name_col_width: u16) -> u16 {
        match self {
            Column::Name => name_col_width,
            Column::SizeAbs => 10,
            Column::SizeRel => 8,
            Column::SizeCur => 8,
            Column::ModTime => 19,
        }
    }

    pub fn right_aligned(self) -> bool {
        match self {
            Column::Name | Column::ModTime => false,
            Column::SizeAbs | Column::SizeRel | Column::SizeCur => true,
        }
    }
}

/// The columns of the file table in display order for a given display state.
pub fn table_columns(show_size_cur: bool) -> Vec<Column> {
    let mut cols = Vec::new();
    cols.push(Column::Name);
    cols.push(Column::SizeAbs);
    cols.push(Column::SizeRel);
    if show_size_cur {
        cols.push(Column::SizeCur);
    }
    cols.push(Column::ModTime);
    cols
}

/// The total width of the file table columns, including the gaps between them.
pub fn columns_width(cols: &[Column], name_col_width: u16) -> u16 {
    cols.iter().map(|c| c.width(name_col_width)).sum::<u16>()
        + COLUMN_GAP.saturating_mul(cols.len().saturating_sub(1) as u16)
}

/// The total width consumed by all columns other than the Name column,
/// including the gaps between the columns. This determines how much terminal
/// width is left for the auto-sized Name column.
pub fn fixed_columns_width(show_size_cur: bool) -> i64 {
    columns_width(&table_columns(show_size_cur), 0) as i64
}

fn render_header(
    frame: &mut Frame,
    state: &State,
    snapshot: &Snapshot,
    theme: &Theme,
    config: &Config,
) -> u16 {
    let style = &theme.header;
    let ri = &snapshot.header;
    let mut line = frame.line(0);

    // Path
    let prefix_str = display_path_prefix(&state.root_dir, &config);
    let suffix_str = state.file_view.cur_subdir.to_string_lossy();
    let path_str = if suffix_str.is_empty() {
        format!("{prefix_str}/")
    } else {
        format!("{prefix_str}/{suffix_str}/")
    };

    let path_label = "Path: ";
    let max_line_len = state.term_size[0]
        .saturating_sub(path_label.len() as u16)
        .max(1) as usize;
    let mut prefix_chars_left = prefix_str.chars().count();
    for (i, path_line) in wrap_path(&path_str, max_line_len).into_iter().enumerate() {
        if i == 0 {
            line.text(&style.label, path_label);
        } else {
            line.spaces(&style.label, path_label.len());
        }
        // Apply the prefix/suffix styles to the parts of the line that belong
        // to the respective part of the path.
        let n_prefix = path_line.chars().count().min(prefix_chars_left);
        let chars: Vec<char> = path_line.chars().collect();
        let prefix_part: String = chars[..n_prefix].iter().collect();
        let suffix_part: String = chars[n_prefix..].iter().collect();
        if !prefix_part.is_empty() {
            line.text(&style.path_prefix, &prefix_part);
        }
        if !suffix_part.is_empty() {
            line.text(&style.path_suffix, &suffix_part);
        }
        prefix_chars_left = prefix_chars_left.saturating_sub(n_prefix);
        line.next_line();
    }

    // Size
    line.text(&style.label, "Size: ");
    if state.file_view.cur_subdir.to_string_lossy().is_empty() {
        let root_dir_size_str = bytes_to_str(ri.root_dir_size);
        line.text(&style.size_total, &root_dir_size_str);
    } else {
        let root_dir_size_str = bytes_to_str(ri.root_dir_size);
        let cur_dir_size_str = bytes_to_str(ri.cur_dir_size);
        line.text(&style.size_current, &cur_dir_size_str);
        line.text(&style.label, " / ");
        line.text(&style.size_total, &root_dir_size_str);
        line.text(&style.label, " (");
        match ratio_to_percent(ri.cur_dir_size, ri.root_dir_size) {
            Some(size_percent) => {
                let percent_str = format!("{size_percent:.2} %");
                if let Some(s) = &style.size_percent {
                    line.text(s, &percent_str);
                } else {
                    let s = Style::default().fg(percent_to_color(size_percent, theme));
                    line.text(&s, &percent_str);
                }
            }
            None => line.text(&style.label, "-"),
        }
        line.text(&style.label, ")");
    }

    // Size Metric
    line.text(&style.label, "  Metric: ");
    line.text(&style.size_metric, snapshot.params.size_metric.as_str());

    // Sort Mode
    line.text(&style.label, "  Sort by ");
    line.text(&style.sort_mode, &format!("{:?}", snapshot.params.sort_by));

    // Errors
    if ri.num_errors > 0 {
        let err_str = if ri.num_errors == 1 {
            "error"
        } else {
            "errors"
        };
        line.text(&style.label, "  ");
        line.text(&style.error, &format!("{} {}", ri.num_errors, err_str));
    }

    // Spinner
    if matches!(ri.root_dir_state, FileState::Queued) {
        line.text(&style.label, "  ");
        line.text(&style.spinner, spinner(state.spinner_frame));
    }

    line.y() + 1
}

fn display_path_prefix(path: &Path, config: &Config) -> String {
    if config.display_prefix_path {
        if config.display_home_as_tilde
            && let Some(home) = dirs::home_dir()
            && let Ok(rest) = path.strip_prefix(&home)
        {
            if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.to_string_lossy())
            }
        } else {
            path.to_string_lossy().into_owned()
        }
    } else {
        "…".to_string()
    }
}

/// Wraps `path` into lines of at most `max_line_len` chars, preferring to
/// break after a '/' character and falling back to a character boundary if a
/// single path component is too long to fit.
fn wrap_path(path: &str, max_line_len: usize) -> Vec<String> {
    assert!(max_line_len > 0, "max_line_len must be greater than 0");
    let chars: Vec<char> = path.chars().collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end_limit = (start + max_line_len).min(chars.len());
        if end_limit == chars.len() {
            lines.push(chars[start..].iter().collect());
            break;
        }
        let mut cut = end_limit;
        for i in ((start + 1)..end_limit).rev() {
            if chars[i] == '/' {
                cut = i + 1;
                break;
            }
        }
        lines.push(chars[start..cut].iter().collect());
        start = cut;
    }
    lines
}

fn render_file_tree(
    frame: &mut Frame,
    state: &State,
    snapshot: &Snapshot,
    theme: &Theme,
    layout: &Layout,
) {
    let ri = &snapshot.header;
    let show_size_cur = state.file_view.show_size_cur();
    let name_col_width = if state.file_view.name_col_width > 0 {
        state.file_view.name_col_width
    } else {
        state
            .file_view
            .auto_name_col_width(state.term_size[0] as i64)
    } as u16;
    let cols = table_columns(show_size_cur);

    // Header
    let style = &theme.file_table;
    {
        let mut line = frame.line(layout.content_top_line());
        let mut x = 0;
        for (i, col) in cols.iter().enumerate() {
            let header_style = match col {
                Column::Name => &style.name_header,
                Column::SizeAbs => &style.size_abs_header,
                Column::SizeRel => &style.size_rel_header,
                Column::SizeCur => &style.size_cur_header,
                Column::ModTime => &style.modtime_header,
            };
            if i > 0 {
                line.spaces_until(&Style::default(), x);
            }
            if col.right_aligned() {
                line.spaces_until(
                    &Style::default(),
                    x + col
                        .width(name_col_width)
                        .saturating_sub(col.header().len() as u16),
                );
            }
            line.text(header_style, col.header());
            x += col.width(name_col_width) + COLUMN_GAP;
        }
    }

    // Rows
    let sel_ix = state
        .file_view
        .cur_entry(snapshot)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let skip = i64::max(0, sel_ix as i64 - (layout.num_content_lines() as i64 / 2)) as usize;
    let row_width = columns_width(&cols, name_col_width);
    for (j, (i, e)) in snapshot.entries.iter().enumerate().skip(skip).enumerate() {
        let y_cur = layout.content_top_line() + 1 + j as u16;
        if y_cur > layout.content_bottom_line() {
            break;
        }
        let is_selected = i == sel_ix;
        let has_error = e.num_child_errors > 0 || matches!(e.node_state, FileState::Failed(_));
        let highlighted = is_selected || has_error;

        let style_mod = |s: &Style| match (is_selected, has_error) {
            (true, true) => s.merge(&style.selected_error_row),
            (true, false) => s.merge(&style.selected_row),
            (false, true) => s.merge(&style.error_row),
            (false, false) => s.clone(),
        };

        let bg_style = style_mod(&Style::default());

        // Paint the full row background first, so that selected and error rows are highlighted
        // across all cells and gaps.
        let mut line = frame.line(y_cur);
        if highlighted {
            line.background(&bg_style, row_width as usize);
        }

        let mut x = 0;
        for (k, col) in cols.iter().enumerate() {
            let width = col.width(name_col_width);
            if k > 0 {
                line.spaces_until(&bg_style, x);
            }
            match col {
                Column::Name => {
                    let mut name = e
                        .name
                        .to_string_lossy()
                        .chars()
                        .take(width as usize)
                        .collect::<String>();
                    if e.is_dir {
                        name += "/";
                    }
                    let base = if e.inode.is_dir {
                        &style.name_dir_value
                    } else if e.inode.num_hardlinks > 1 {
                        &style.name_hardlink_value
                    } else {
                        &style.name_file_value
                    };
                    let s = if matches!(e.node_state, FileState::Queued) {
                        base.merge(&style.name_scan_value)
                    } else {
                        base.clone()
                    };
                    line.text(&style_mod(&s), &name);
                }
                Column::SizeAbs => {
                    let size_abs_str = bytes_to_str(e.size);
                    line.spaces_until(
                        &bg_style,
                        x + width.saturating_sub(size_abs_str.len() as u16),
                    );
                    line.text(&style_mod(&style.size_abs_value), &size_abs_str);
                }
                Column::SizeRel => {
                    let abs_size_percent = ratio_to_percent(e.size, ri.root_dir_size);
                    let abs_size_percent_str = match abs_size_percent {
                        Some(p) => format!("{p:6.2} %"),
                        None => "       -".to_string(),
                    };
                    line.spaces_until(
                        &bg_style,
                        x + width.saturating_sub(abs_size_percent_str.len() as u16),
                    );
                    let s = if let Some(s) = &style.size_rel_value {
                        style_mod(s)
                    } else {
                        let mut s = bg_style.clone();
                        if let Some(p) = abs_size_percent
                            && !has_error
                        {
                            s = s.fg(percent_to_color(p, theme));
                        }
                        s
                    };
                    line.text(&s, &abs_size_percent_str);
                }
                Column::SizeCur => {
                    let size_percent = ratio_to_percent(e.size, ri.cur_dir_size);
                    let size_percent_str = match size_percent {
                        Some(p) => format!("{p:6.2} %"),
                        None => "       -".to_string(),
                    };
                    line.spaces_until(
                        &bg_style,
                        x + width.saturating_sub(size_percent_str.len() as u16),
                    );
                    let s = if let Some(s) = &style.size_cur_value {
                        style_mod(s)
                    } else {
                        let mut s = bg_style.clone();
                        if let Some(p) = size_percent {
                            s = s.fg(percent_to_color(p, theme));
                        }
                        s
                    };
                    line.text(&s, &size_percent_str);
                }
                Column::ModTime => {
                    let s_val = &style_mod(&style.modtime_value);
                    let s_delim = &style_mod(&style.modtime_delim);
                    match e.inode.last_modified {
                        Some(last_modified) => {
                            let last_modified = DateTime::<Local>::from(last_modified);
                            line.text(s_val, &last_modified.format("%Y").to_string());
                            line.text(s_delim, "-");
                            line.text(s_val, &last_modified.format("%m").to_string());
                            line.text(s_delim, "-");
                            line.text(s_val, &last_modified.format("%d %H").to_string());
                            line.text(s_delim, ":");
                            line.text(s_val, &last_modified.format("%M").to_string());
                            line.text(s_delim, ":");
                            line.text(s_val, &last_modified.format("%S").to_string());
                        }
                        // The modtime could not be retrieved, so display a placeholder.
                        None => line.text(s_val, "-"),
                    }
                }
            }
            x += width + COLUMN_GAP;
        }
    }
}

fn render_delete_dialog(frame: &mut Frame, theme: &Theme, layout: &Layout, path_to_delete: &Path) {
    let style = &theme.delete_dialog;
    let mut line = frame.line(layout.content_top_line());
    line.text(
        &style.message,
        "Delete the following file/directory?  [y/n]",
    );
    let mut line = frame.line(layout.content_top_line() + 2);
    line.spaces_until(&Style::default(), 2);
    line.text(&style.path, path_to_delete.to_string_lossy().as_ref());
}

fn render_errors(
    frame: &mut Frame,
    state: &State,
    theme: &Theme,
    error_view: &ErrorView,
    layout: &Layout,
) {
    let style = &theme.error_view;

    let width = state.term_size[0] as usize;
    let mut y = layout.content_top_line();
    for (i, e) in error_view
        .errors
        .iter()
        .enumerate()
        .skip(error_view.cur_error_ix)
    {
        {
            let mut line = frame.line(y);
            line.text(&style.index, &(i + 1).to_string());
            line.spaces_until(&Style::default(), 4);
            line.text(&style.path, &e.path);
        }
        y += 1;
        for l in wrap_lines(&e.message, width.saturating_sub(4).max(10)) {
            let mut line = frame.line(y);
            line.spaces_until(&Style::default(), 4);
            line.text(&style.message, &l);
            y += 1;
        }
        y += 1;
        if y > layout.content_bottom_line() {
            break;
        }
    }
}

fn render_help_view(
    frame: &mut Frame,
    state: &State,
    theme: &Theme,
    help_view: &HelpView,
    layout: &Layout,
) {
    let style = &theme.help_view;

    for (i, (keys, action, scope)) in HELP.iter().skip(help_view.cur_line_ix).enumerate() {
        let y = layout.content_top_line() + i as u16;
        if y > layout.content_bottom_line() {
            break;
        }
        let (s_keys, s_action, s_scope) = if i == 0 {
            (
                &style.keys_header,
                &style.action_header,
                &style.scope_header,
            )
        } else {
            (&style.keys, &style.action, &style.scope)
        };
        let mut line = frame.line(y);
        line.text(s_keys, keys);
        line.spaces(s_keys, state.help_meta.key_col_width + 2 - keys.len());
        line.text(s_action, action);
        line.spaces(
            s_action,
            state.help_meta.action_col_width + 2 - action.len(),
        );
        line.text(s_scope, scope);
    }
}

/// Wraps `input` into lines that are at most `max_line_length` chars long.
///
/// Existing newlines in `input` are preserved. Any single input line longer
/// than `max_line_length` is broken at a word boundary if possible, otherwise
/// in the middle of a word. Continuation lines produced by breaking a single
/// input line are indented by two spaces, and the indentation counts toward
/// `max_line_length`.
pub fn wrap_lines(input: &str, max_line_length: usize) -> Vec<String> {
    assert!(
        max_line_length > 0,
        "max_line_length must be greater than 0"
    );

    let mut lines = Vec::new();
    for line in input.split('\n') {
        // Be lenient with CRLF input.
        let line = line.strip_suffix('\r').unwrap_or(line);
        wrap_single_line(line, max_line_length, &mut lines);
    }
    lines
}

fn wrap_single_line(line: &str, max_line_length: usize, lines: &mut Vec<String>) {
    if line.trim().is_empty() {
        lines.push(String::new());
        return;
    }

    let indent = if max_line_length > 2 { "  " } else { "" };

    let mut rest: Vec<char> = line.chars().collect();
    let mut first = true;

    while !rest.is_empty() {
        let budget = max_line_length - (if first { 0 } else { indent.len() });

        let mut cut = budget.min(rest.len());
        if cut < rest.len() {
            let mut boundary: Option<usize> = None;
            let mut seen_content = false;
            for (i, &c) in rest.iter().take(cut).enumerate() {
                if c.is_whitespace() {
                    if seen_content {
                        boundary = Some(i);
                    }
                } else {
                    seen_content = true;
                }
            }
            if let Some(i) = boundary {
                cut = i;
            }
        }

        let segment: String = rest[..cut].iter().collect();
        let segment = segment.trim_end();
        lines.push(if first {
            segment.to_string()
        } else {
            format!("{indent}{segment}")
        });

        rest = rest[cut..].to_vec();
        let skipped = rest.iter().take_while(|c| c.is_whitespace()).count();
        rest.drain(..skipped);
        first = false;
    }
}

pub fn spinner(i: usize) -> &'static str {
    match i % 6 {
        0 => "... ",
        1 => ".. .",
        2 => ". ..",
        3 => " ...",
        4 => ". ..",
        5 => ".. .",
        _ => unreachable!(),
    }
}

pub fn ratio_to_percent(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64 * 100.0)
}

pub fn percent_to_color(f: f64, theme: &Theme) -> [f32; 3] {
    let f = (f / 100.0) as f32;
    let f = f.sqrt();
    interpolate_color(f, theme.size_min_color, theme.size_max_color)
}

pub fn bytes_to_str(mut n: u64) -> String {
    let mut unit = 0;
    let mut rest = 0;
    while n > 1024 {
        unit += 1;
        rest = n % 1024;
        n /= 1024;
    }

    let size = n as f32 + (rest as f32 / 1024.0);
    let unit = match unit {
        0 => "B",
        1 => "KB",
        2 => "MB",
        3 => "GB",
        4 => "TB",
        5 => "PB",
        6 => "EB",
        _ => "?B",
    };

    if unit == "B" {
        format!("{} {}", size, unit)
    } else {
        format!("{:.2} {}", size, unit)
    }
}

pub fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb {
        r: (r * 255.0) as u8,
        g: (g * 255.0) as u8,
        b: (b * 255.0) as u8,
    }
}
