use std::{
    ffi::OsString,
    io::{Stdout, Write},
    path::PathBuf,
};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::{
    cursor,
    style::{self, Color, ResetColor},
    terminal::{Clear, ClearType},
};

use crate::{
    DisplayMode, ErrorView, ErrorViewMode, SortBy, State,
    file_tree::{FileNode, FileState, FileTree},
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
) -> Vec<(&'a OsString, &'a FileNode, Inode, bool)> {
    let children = &file_tree.get(&state.cur_subdir).unwrap().children;
    let mut children = children
        .iter()
        .map(|(name, node)| {
            let path = state.target_path.join(&state.cur_subdir).join(name);
            let is_dir = path.is_dir() && !path.is_symlink();
            let inode = *file_tree.inodes.get(&node.inode_id).unwrap();
            (name, node, inode, is_dir)
        })
        .collect::<Vec<_>>();
    match state.sort_by {
        SortBy::Name => {
            children.sort_by_key(|(name, _node, _inode, is_dir)| {
                let is_file = !*is_dir && state.sort_dirs_before_files;
                (is_file, *name)
            });
        }
        SortBy::Size => {
            children.sort_by_key(|(_name, _node, inode, _is_dir)| {
                // let is_file = !*is_dir && state.sort_dirs_before_files;
                // (is_file, inode.size_in_bytes)
                inode.size_in_bytes
            });
            children.reverse();
        }
        SortBy::ModTime => {
            children.sort_by_key(|(_name, _node, inode, _is_dir)| {
                // let is_file = !*is_dir && state.sort_dirs_before_files;
                // (is_file, inode.last_modified)
                inode.last_modified
            });
            children.reverse();
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
        RenderInfo {
            cur_dir_size: file_tree
                .get(&state.cur_subdir)
                .and_then(|n| file_tree.inodes.get(&n.inode_id).map(|n| n.size_in_bytes))
                .unwrap_or(0),
            target_dir_size: file_tree
                .get(&PathBuf::from(""))
                .and_then(|n| file_tree.inodes.get(&n.inode_id).map(|n| n.size_in_bytes))
                .unwrap_or(0),
            target_dir_state: file_tree
                .get(&PathBuf::from(""))
                .as_ref()
                .unwrap()
                .state
                .clone(),
            num_errors: file_tree.errors.len(),
        }
    };

    render_header(&mut w, &state, theme, &render_info)?;
    match &state.display_mode {
        DisplayMode::FileTree => render_file_tree(&mut w, state, theme, &render_info)?,
        DisplayMode::ErrorView(error_view) => render_errors(&mut w, &state, theme, error_view)?,
    };

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

fn render_header(w: &mut impl Write, state: &State, theme: &Theme, ri: &RenderInfo) -> Result<()> {
    write!(w, "{}", cursor::MoveTo(0, 0))?;
    let style = &theme.header;

    // Current Directory
    let prefix_str = state.target_path.to_string_lossy();
    let suffix_str = state.cur_subdir.to_string_lossy();
    style.label.write_text(w, "Current Directory: ")?;
    style.path_prefix.write_text(w, prefix_str.as_ref())?;
    style.label.write_text(w, "/")?;
    style.path_suffix.write_text(w, suffix_str.as_ref())?;
    style.label.write_text(w, ", ")?;

    // Size
    if state.cur_subdir.to_string_lossy().is_empty() {
        let target_size_str = bytes_to_str(ri.target_dir_size);
        style.label.write_text(w, "Size: ")?;
        style.size_total.write_text(w, &target_size_str)?;
    } else {
        let size_percent = (ri.cur_dir_size as f64 / ri.target_dir_size as f64) * 100.0;
        let target_size_str = bytes_to_str(ri.target_dir_size);
        let cur_dir_size_str = bytes_to_str(ri.cur_dir_size);
        style.label.write_text(w, "Size: ")?;
        style.size_current.write_text(w, &cur_dir_size_str)?;
        style.label.write_text(w, " / ")?;
        style.size_total.write_text(w, &target_size_str)?;
        style.label.write_text(w, " (")?;
        if let Some(s) = &style.size_percent {
            s.write_text(w, &format!("{size_percent:.2}"))?;
        } else {
            let s = style::SetForegroundColor(color_to_crossterm(&percent_to_color(
                size_percent,
                theme,
            )));
            write!(w, "{s}{size_percent:.2}{}", ResetColor)?;
        }
        style.label.write_text(w, ")")?;
    }
    style.label.write_text(w, ", ")?;

    // Sort Mode
    style.label.write_text(w, "Sort by ")?;
    style
        .sort_mode
        .write_text(w, &format!("{:?}", state.sort_by))?;

    // Errors
    if ri.num_errors > 0 {
        style.label.write_text(w, ", ")?;
        let err_str = if ri.num_errors == 1 {
            "error"
        } else {
            "errors"
        };
        style
            .error
            .write_text(w, &format!("{} {}", ri.num_errors, err_str))?;
    }

    // Progress State
    match &ri.target_dir_state {
        FileState::Queued | FileState::InProgress => {
            style.label.write(w)?;
            write!(w, ", Scanning {}", loading_text(state.frame))?;
        }
        FileState::Finished => (),
        FileState::Failed(err) => {
            style.label.write(w)?;
            write!(
                w,
                ", {}ERROR: {}{}",
                style::SetForegroundColor(rgb(1.0, 0.0, 0.0)),
                err,
                style::ResetColor,
            )?;
        }
    }

    Ok(())
}

fn render_file_tree(
    w: &mut impl Write,
    state: &mut State,
    theme: &Theme,
    ri: &RenderInfo,
) -> Result<()> {
    let is_scan_done = match ri.target_dir_state {
        FileState::Queued | FileState::InProgress => false,
        FileState::Finished | FileState::Failed(_) => true,
    };

    // File View Header
    write!(w, "{}", cursor::MoveTo(0, 2))?;
    let style = &theme.file_table;
    if !is_scan_done {
        style.progress_header.write_text(w, "S")?;
        write!(w, "  ")?;
    }
    let name_col_width = i64::max(
        8,
        i64::min(
            32,
            state.term_size[0] as i64 - 46 - if is_scan_done { 0 } else { 3 },
        ),
    );
    style.name_header.write_text(w, "Name")?;
    write!(w, "  ")?;
    for _ in 0..name_col_width - 4 {
        write!(w, " ")?;
    }

    write!(w, "  ")?;
    style.size_abs_header.write_text(w, "Size Abs")?;
    write!(w, "  ")?;

    write!(w, "  ")?;
    style.size_rel_header.write_text(w, "Size Rel")?;
    write!(w, "  ")?;

    if !state.cur_subdir.to_string_lossy().is_empty() {
        write!(w, "  ")?;
        style.size_cur_header.write_text(w, "Size Cur")?;
        write!(w, "  ")?;
    }
    style.modtime_header.write_text(w, "Last Modified")?;

    // File View Rows
    let file_tree = state.file_tree.lock().unwrap();
    let children = sorted_entries(&file_tree, state);
    let skip = i64::max(
        0,
        state.cur_subdir_entry_ix() as i64 - ((state.term_size[1] as i64 - 3) / 2),
    ) as usize;
    for (j, (i, (name, node, inode, is_dir))) in children.iter().enumerate().skip(skip).enumerate()
    {
        write!(w, "{}", cursor::MoveTo(0, j as u16 + 3))?;
        let is_selected = i == state.cur_subdir_entry_ix();
        let mut x = 0;

        let maybe_selected = |s: &Style| {
            if is_selected {
                s.merge(&style.selected_row)
            } else {
                s.clone()
            }
        };

        let bg_style = maybe_selected(&Style::default());

        // State
        if !is_scan_done {
            let state_str = match node.state {
                FileState::Queued | FileState::InProgress => "…",
                FileState::Finished => " ",
                FileState::Failed(_) => "E",
            };
            maybe_selected(&style.progress_value).write_text(w, state_str)?;
            bg_style.write_text(w, "  ")?;
            x += 3;
        }

        // File Name
        let mut name = name
            .to_string_lossy()
            .chars()
            .take(name_col_width as usize)
            .collect::<String>();
        if *is_dir {
            name += "/";
        }
        let s = if inode.is_dir {
            &style.name_dir_value
        } else {
            &style.name_file_value
        };
        maybe_selected(s).write_text(w, &name)?;
        x += name_col_width;

        // Make sure background color is printed even if name is shorter than max length.
        if is_selected {
            let spaces: String = (name.chars().count()..name_col_width as usize)
                .map(|_| " ")
                .collect();
            bg_style.write_text(w, &spaces)?;
        } else {
            write!(w, "{}", cursor::MoveTo(x as u16, j as u16 + 3))?;
        };
        bg_style.write_text(w, "  ")?;

        // Size Abs
        let size_abs_str = bytes_to_str(inode.size_in_bytes);
        let spaces: String = (size_abs_str.len()..10).map(|_| " ").collect();
        bg_style.write_text(w, &spaces)?;
        maybe_selected(&style.size_abs_value).write_text(w, &size_abs_str)?;
        bg_style.write_text(w, "  ")?;

        // Size Rel
        let abs_size_percent = (inode.size_in_bytes as f64 / ri.target_dir_size as f64) * 100.0;
        let abs_size_percent_str = format!("{:6.2} %", abs_size_percent);
        let spaces: String = (abs_size_percent_str.len()..10).map(|_| " ").collect();
        bg_style.write_text(w, &spaces)?;
        if let Some(s) = &style.size_rel_value {
            s.write(w)?;
        } else {
            bg_style
                .clone()
                .fg(percent_to_color(abs_size_percent, theme))
                .write(w)?;
        }
        write!(w, "{}", abs_size_percent_str)?;
        write!(w, "{}", style::ResetColor)?;
        bg_style.write_text(w, "  ")?;

        // Size Cur
        if !state.cur_subdir.to_string_lossy().is_empty() {
            let size_percent = (inode.size_in_bytes as f64 / ri.cur_dir_size as f64) * 100.0;
            let size_percent_str = format!("{:6.2} %", size_percent);
            let spaces: String = (abs_size_percent_str.len()..10).map(|_| " ").collect();
            bg_style.write_text(w, &spaces)?;
            if let Some(s) = &style.size_cur_value {
                s.write(w)?;
            } else {
                bg_style
                    .clone()
                    .fg(percent_to_color(abs_size_percent, theme))
                    .write(w)?;
            }
            write!(w, "{}", size_percent_str)?;
            write!(w, "{}", style::ResetColor)?;
            bg_style.write_text(w, "  ")?;
        }

        // Last Modified
        let last_modified = DateTime::<Local>::from(inode.last_modified);
        let s_val = format!(
            "{}{}",
            ResetColor,
            maybe_selected(&style.modtime_value).to_string()
        );
        let s_delim = format!(
            "{}{}",
            ResetColor,
            maybe_selected(&style.modtime_delim).to_string()
        );
        let last_modified = last_modified
            .format("%Y[-]%m[-]%d %H[:]%M[:]%S")
            .to_string()
            .replace("[", &s_delim)
            .replace("]", &s_val);
        write!(w, "{}", s_val)?;
        write!(w, "{}", last_modified)?;
        write!(w, "{}", style::ResetColor)?;
    }

    Ok(())
}

fn render_errors(
    w: &mut impl Write,
    state: &State,
    theme: &Theme,
    error_view: &ErrorView,
) -> Result<()> {
    let style = &theme.error_view;

    let mut y = 2;
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
        if y > state.term_size[1] {
            break;
        }
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
