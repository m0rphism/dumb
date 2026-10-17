use std::{
    ffi::OsString,
    io::{Stdout, Write},
    path::Path,
};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::{
    cursor,
    style::{self, Color, ResetColor},
    terminal::{Clear, ClearType},
};

use crate::{
    DisplayMode, ErrorView, HelpView, SortBy, State,
    config::{HeaderLayout, SizeMetric},
    file_tree::{FileNode, FileState, FileTree},
    help::HELP,
    scan::Inode,
    theme::{Style, Theme, color_to_crossterm, interpolate_color},
};

pub fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb {
        r: (r * 255.0) as u8,
        g: (g * 255.0) as u8,
        b: (b * 255.0) as u8,
    }
}

pub fn sorted_entries<'a>(
    file_tree: &'a FileTree,
    state: &State,
) -> Vec<(&'a OsString, &'a FileNode, Inode, bool, u64)> {
    let children = &file_tree.get(&state.cur_subdir).unwrap().children;
    let mut children = children
        .iter()
        .map(|(name, node)| {
            let path = state.target_path.join(&state.cur_subdir).join(name);
            let is_dir = path.is_dir() && !path.is_symlink();
            let inode = *file_tree.inodes.get(&node.inode_id).unwrap();
            let size = match state.size_metric {
                SizeMetric::Naive => {
                    if inode.is_dir {
                        node.naive_size
                    } else {
                        inode.allocated_bytes
                    }
                }
                SizeMetric::Freed => {
                    if inode.is_dir {
                        node.freed_size
                    } else if inode.num_hardlinks == 1 {
                        inode.allocated_bytes
                    } else {
                        0
                    }
                }
            };
            (name, node, inode, is_dir, size)
        })
        .collect::<Vec<_>>();
    match state.sort_by {
        SortBy::Name => {
            children.sort_by_key(|(name, _node, _inode, is_dir, _size)| {
                let is_file = !*is_dir && state.sort_dirs_before_files;
                (is_file, *name)
            });
        }
        SortBy::Size => {
            children.sort_by(
                |(name1, _node1, _inode1, _dir1, size1), (name2, _node2, _inode2, _dir2, size2)| {
                    (size2, *name1).cmp(&(size1, *name2))
                },
            );
        }
        SortBy::ModTime => {
            children.sort_by(
                |(name1, _node1, inode1, _dir1, _size1), (name2, _node2, inode2, _dir2, _size2)| {
                    (inode2.last_modified, *name1).cmp(&(inode1.last_modified, *name2))
                },
            );
        }
    }
    children
}

pub fn render_state(stdout: &mut Stdout, state: &mut State, theme: &Theme) -> Result<()> {
    // We buffer all rendering before sending it to stdout to remove terminal flickering.
    // Flickering shouldn't be an issue if stdout is only flushed at the end, but for some reason
    // it is...
    let mut w = vec![];

    write!(w, "{}", Clear(ClearType::All))?;

    let render_info = {
        let file_tree = state.file_tree.lock().unwrap();
        let size_of = |n: &FileNode| match state.size_metric {
            SizeMetric::Freed => n.freed_size,
            SizeMetric::Naive => n.naive_size,
        };
        RenderInfo {
            cur_dir_size: file_tree.get(&state.cur_subdir).map(size_of).unwrap_or(0),
            target_dir_size: size_of(&file_tree.root),
            target_dir_state: file_tree.root.state.clone(),
            num_errors: file_tree.errors.len(),
        }
    };

    let header_height = render_header(&mut w, state, theme, &render_info)?;
    match &state.display_mode {
        DisplayMode::FileTree => {
            render_file_tree(&mut w, state, theme, &render_info, header_height + 1)?
        }
        DisplayMode::ErrorView(error_view) => {
            render_errors(&mut w, state, theme, error_view, header_height + 1)?
        }
        DisplayMode::HelpView(help_view) => {
            render_help_view(&mut w, state, theme, help_view, header_height + 1)?
        }
        DisplayMode::DeleteDialog(target) => {
            render_delete_dialog(
                &mut w,
                theme,
                header_height + 1,
                &state.target_path.join(target),
            )?;
        }
    };

    // Status message (e.g. a failed deletion) on the bottom line.
    if let Some(msg) = &state.status {
        let width = state.term_size[0].saturating_sub(1) as usize;
        let msg: String = msg.chars().take(width).collect();
        write!(
            w,
            "{}",
            cursor::MoveTo(0, state.term_size[1].saturating_sub(1))
        )?;
        theme.header.error.write_text(&mut w, &msg)?;
    }

    state.frame += 1;

    stdout.write_all(&w)?;
    stdout.flush()?;

    Ok(())
}

pub struct RenderInfo {
    pub cur_dir_size: u64,
    pub target_dir_size: u64,
    pub target_dir_state: FileState,
    pub num_errors: usize,
}

/// The number of spaces between two adjacent columns of the file table.
const COLUMN_GAP: u16 = 2;

/// A column of the file table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// Scan progress indicator. Only visible while the scan is running.
    Progress,
    /// Entry name. The only column whose width is not fixed.
    Name,
    /// Absolute size of the entry.
    SizeAbs,
    /// Size of the entry relative to the scan target.
    SizeRel,
    /// Size of the entry relative to the current directory.
    SizeCur,
    /// Last modification time of the entry.
    ModTime,
}

impl Column {
    pub fn header(self) -> &'static str {
        match self {
            Column::Progress => "S",
            Column::Name => "Name",
            Column::SizeAbs => "Size Abs",
            Column::SizeRel => "Size Rel",
            Column::SizeCur => "Size Cur",
            Column::ModTime => "Last Modified",
        }
    }

    pub fn width(self, name_col_width: u16) -> u16 {
        match self {
            Column::Progress => 1,
            Column::Name => name_col_width,
            Column::SizeAbs => 10,
            Column::SizeRel => 8,
            Column::SizeCur => 8,
            Column::ModTime => 19,
        }
    }

    pub fn right_aligned(self) -> bool {
        match self {
            Column::Progress | Column::Name | Column::ModTime => false,
            Column::SizeAbs | Column::SizeRel | Column::SizeCur => true,
        }
    }
}

/// The columns of the file table in display order for a given display state.
pub fn table_columns(is_scan_done: bool, show_size_cur: bool) -> Vec<Column> {
    let mut cols = Vec::new();
    if !is_scan_done {
        cols.push(Column::Progress);
    }
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
pub fn fixed_columns_width(is_scan_done: bool, show_size_cur: bool) -> i64 {
    columns_width(&table_columns(is_scan_done, show_size_cur), 0) as i64
}

fn render_header<W: Write>(
    w: &mut W,
    state: &State,
    theme: &Theme,
    ri: &RenderInfo,
) -> Result<u16> {
    write!(w, "{}", cursor::MoveTo(0, 0))?;
    let style = &theme.header;

    let mut y = 0;

    // Current Directory
    let prefix_str = state.target_path.to_string_lossy();
    let suffix_str = state.cur_subdir.to_string_lossy();

    let next_field = |y: &mut u16, w: &mut W| -> Result<()> {
        match state.header_layout {
            HeaderLayout::SingleLine => style.label.write_text(w, "  ")?,
            HeaderLayout::MultiLine => {
                *y += 1;
                write!(w, "{}", cursor::MoveTo(0, *y))?;
            }
        }
        Ok(())
    };

    if state.header_layout == HeaderLayout::MultiLine {
        style.label.write_text(w, "Cur Dir:  ")?;
    }
    style.path_prefix.write_text(w, prefix_str.as_ref())?;
    style.path_prefix.write_text(w, "/")?;
    style.path_suffix.write_text(w, suffix_str.as_ref())?;
    next_field(&mut y, w)?;

    // Size
    if state.header_layout == HeaderLayout::MultiLine {
        style.label.write_text(w, "Size:     ")?;
    }
    if state.cur_subdir.to_string_lossy().is_empty() {
        let target_size_str = bytes_to_str(ri.target_dir_size);
        style.size_total.write_text(w, &target_size_str)?;
    } else {
        let size_percent = (ri.cur_dir_size as f64 / ri.target_dir_size as f64) * 100.0;
        let target_size_str = bytes_to_str(ri.target_dir_size);
        let cur_dir_size_str = bytes_to_str(ri.cur_dir_size);
        style.size_current.write_text(w, &cur_dir_size_str)?;
        style.label.write_text(w, " / ")?;
        style.size_total.write_text(w, &target_size_str)?;
        style.label.write_text(w, " (")?;
        if let Some(s) = &style.size_percent {
            s.write_text(w, &format!("{size_percent:.2} %"))?;
        } else {
            let s = style::SetForegroundColor(color_to_crossterm(&percent_to_color(
                size_percent,
                theme,
            )));
            write!(w, "{s}{size_percent:.2} %{}", ResetColor)?;
        }
        style.label.write_text(w, ")")?;
    }
    style.label.write_text(w, " [")?;
    style
        .size_metric
        .write_text(w, state.size_metric.as_str())?;
    style.label.write_text(w, "]")?;
    next_field(&mut y, w)?;

    // Sort Mode
    match state.header_layout {
        HeaderLayout::SingleLine => style.label.write_text(w, "Sort by ")?,
        HeaderLayout::MultiLine => style.label.write_text(w, "Sort by:  ")?,
    }
    style
        .sort_mode
        .write_text(w, &format!("{:?}", state.sort_by))?;

    // Errors
    if ri.num_errors > 0 {
        next_field(&mut y, w)?;
        match state.header_layout {
            HeaderLayout::SingleLine => {
                let err_str = if ri.num_errors == 1 {
                    "error"
                } else {
                    "errors"
                };
                style
                    .error
                    .write_text(w, &format!("{} {}", ri.num_errors, err_str))?;
            }
            HeaderLayout::MultiLine => {
                style.label.write_text(w, "Errors:   ")?;
                style.error.write_text(w, &format!("{}", ri.num_errors))?;
            }
        }
    }

    // Spinner
    if matches!(ri.target_dir_state, FileState::Queued) {
        next_field(&mut y, w)?;
        style.spinner.write(w)?;
        write!(w, "{}", loading_text(state.frame))?;
    }

    Ok(y + 1)
}

fn render_file_tree(
    w: &mut impl Write,
    state: &mut State,
    theme: &Theme,
    ri: &RenderInfo,
    mut y: u16,
) -> Result<()> {
    let is_scan_done = match ri.target_dir_state {
        FileState::Queued => false,
        FileState::Finished | FileState::Failed(_) => true,
    };
    let show_size_cur = state.show_size_cur();
    let name_col_width = if state.name_col_width > 0 {
        state.name_col_width
    } else {
        state.auto_name_col_width()
    } as u16;
    let cols = table_columns(is_scan_done, show_size_cur);

    // Header
    let style = &theme.file_table;
    let mut x = 0;
    for col in &cols {
        write!(w, "{}", cursor::MoveTo(x, y))?;
        let header_style = match col {
            Column::Progress => &style.progress_header,
            Column::Name => &style.name_header,
            Column::SizeAbs => &style.size_abs_header,
            Column::SizeRel => &style.size_rel_header,
            Column::SizeCur => &style.size_cur_header,
            Column::ModTime => &style.modtime_header,
        };
        if col.right_aligned() {
            let pad = col
                .width(name_col_width)
                .saturating_sub(col.header().len() as u16) as usize;
            write!(w, "{}", " ".repeat(pad))?;
        }
        header_style.write_text(w, col.header())?;
        x += col.width(name_col_width) + COLUMN_GAP;
    }

    y += 1;

    // Rows
    let file_tree = state.file_tree.lock().unwrap();
    let children = sorted_entries(&file_tree, state);
    let skip = i64::max(
        0,
        state.cur_subdir_entry_ix() as i64 - ((state.term_size[1] as i64 - y as i64) / 2),
    ) as usize;
    let row_width = columns_width(&cols, name_col_width);
    for (j, (i, (name, node, inode, is_dir, size))) in
        children.iter().enumerate().skip(skip).enumerate()
    {
        if y as usize + j >= state.term_size[1] as usize {
            break;
        }
        let y_cur = y + j as u16;
        let is_selected = i == state.cur_subdir_entry_ix();
        let has_error = node.num_child_errors > 0 || matches!(node.state, FileState::Failed(_));
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
        if highlighted {
            write!(w, "{}", cursor::MoveTo(0, y_cur))?;
            bg_style.write_spaces(w, row_width as usize)?;
        }

        let mut x = 0;
        for col in &cols {
            let width = col.width(name_col_width) as usize;
            write!(w, "{}", cursor::MoveTo(x, y_cur))?;
            match col {
                Column::Progress => {
                    let state_str = match node.state {
                        FileState::Queued => "…",
                        FileState::Finished => " ",
                        FileState::Failed(_) => "E",
                    };
                    style_mod(&style.progress_value).write_text(w, state_str)?;
                }
                Column::Name => {
                    let mut name = name
                        .to_string_lossy()
                        .chars()
                        .take(width)
                        .collect::<String>();
                    if *is_dir {
                        name += "/";
                    }
                    let s = if inode.is_dir {
                        &style.name_dir_value
                    } else if inode.num_hardlinks > 1 {
                        &style.name_hardlink_value
                    } else {
                        &style.name_file_value
                    };
                    style_mod(s).write_text(w, &name)?;
                }
                Column::SizeAbs => {
                    let size_abs_str = bytes_to_str(*size);
                    bg_style.write_spaces(w, width.saturating_sub(size_abs_str.len()))?;
                    style_mod(&style.size_abs_value).write_text(w, &size_abs_str)?;
                }
                Column::SizeRel => {
                    let abs_size_percent = (*size as f64 / ri.target_dir_size as f64) * 100.0;
                    let abs_size_percent_str = format!("{:6.2} %", abs_size_percent);
                    bg_style.write_spaces(w, width.saturating_sub(abs_size_percent_str.len()))?;
                    if let Some(s) = &style.size_rel_value {
                        style_mod(s).write(w)?;
                    } else {
                        let mut s = bg_style.clone();
                        if !has_error {
                            s = s.fg(percent_to_color(abs_size_percent, theme));
                        }
                        s.write(w)?;
                    }
                    write!(w, "{}", abs_size_percent_str)?;
                    write!(w, "{}", style::ResetColor)?;
                }
                Column::SizeCur => {
                    let size_percent = (*size as f64 / ri.cur_dir_size as f64) * 100.0;
                    let size_percent_str = format!("{:6.2} %", size_percent);
                    bg_style.write_spaces(w, width.saturating_sub(size_percent_str.len()))?;
                    if let Some(s) = &style.size_cur_value {
                        s.write(w)?;
                    } else {
                        bg_style
                            .clone()
                            .fg(percent_to_color(size_percent, theme))
                            .write(w)?;
                    }
                    write!(w, "{}", size_percent_str)?;
                    write!(w, "{}", style::ResetColor)?;
                }
                Column::ModTime => {
                    let last_modified = DateTime::<Local>::from(inode.last_modified);
                    let s_val = format!("{}{}", ResetColor, style_mod(&style.modtime_value));
                    let s_delim = format!("{}{}", ResetColor, style_mod(&style.modtime_delim));
                    let last_modified = last_modified
                        .format("%Y[-]%m[-]%d %H[:]%M[:]%S")
                        .to_string()
                        .replace("[", &s_delim)
                        .replace("]", &s_val);
                    write!(w, "{}", s_val)?;
                    write!(w, "{}", last_modified)?;
                    write!(w, "{}", style::ResetColor)?;
                }
            }
            x += col.width(name_col_width) + COLUMN_GAP;
        }
    }

    Ok(())
}

fn render_delete_dialog(
    w: &mut impl Write,
    theme: &Theme,
    y: u16,
    path_to_delete: &Path,
) -> Result<()> {
    let style = &theme.delete_dialog;
    write!(w, "{}", cursor::MoveTo(0, y))?;
    style
        .message
        .write_text(w, "Delete the following file/directory?  [y/n]")?;
    write!(w, "{}", cursor::MoveTo(2, y + 2))?;
    style
        .path
        .write_text(w, path_to_delete.to_string_lossy().as_ref())?;

    Ok(())
}

fn render_errors(
    w: &mut impl Write,
    state: &State,
    theme: &Theme,
    error_view: &ErrorView,
    mut y: u16,
) -> Result<()> {
    let style = &theme.error_view;

    let width = state.term_size[0] as usize;
    for (i, e) in error_view
        .errors
        .iter()
        .enumerate()
        .skip(error_view.cur_error_ix)
    {
        write!(w, "{}", cursor::MoveTo(0, y))?;
        style.index.write_text(w, &(i + 1).to_string())?;
        write!(w, "{}", cursor::MoveTo(4, y))?;
        style.path.write_text(w, &e.path)?;
        y += 1;
        for l in wrap_lines(&e.message, width.saturating_sub(4).max(10)) {
            write!(w, "{}", cursor::MoveTo(4, y))?;
            style.message.write_text(w, &l)?;
            y += 1;
        }
        y += 1;
        if y >= state.term_size[1] {
            break;
        }
    }

    Ok(())
}

fn render_help_view(
    w: &mut impl Write,
    state: &State,
    theme: &Theme,
    help_view: &HelpView,
    y: u16,
) -> Result<()> {
    let style = &theme.help_view;

    for (i, (keys, action, scope)) in HELP.iter().skip(help_view.cur_line_ix).enumerate() {
        let y = y + i as u16;
        if y >= state.term_size[1] {
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
        write!(w, "{}", cursor::MoveTo(0, y))?;
        s_keys.write_text(w, keys)?;
        s_keys.write_spaces(w, state.help_meta.key_col_width + 2 - keys.len())?;
        s_action.write_text(w, action)?;
        s_action.write_spaces(w, state.help_meta.action_col_width + 2 - action.len())?;
        s_scope.write_text(w, scope)?;
    }

    Ok(())
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

pub fn loading_text(i: usize) -> &'static str {
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
