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
    SortBy, State,
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

    {
        let file_tree = state.file_tree.lock().unwrap();
        let cur_dir_size = file_tree
            .get(&state.cur_subdir)
            .and_then(|n| file_tree.inodes.get(&n.inode_id).map(|n| n.size_in_bytes))
            .unwrap_or(0);
        let target_dir_size = file_tree
            .get(&PathBuf::from(""))
            .and_then(|n| file_tree.inodes.get(&n.inode_id).map(|n| n.size_in_bytes))
            .unwrap_or(0);
        let target_dir_state = &file_tree.get(&PathBuf::from("")).as_ref().unwrap().state;

        write!(w, "{}", cursor::MoveTo(0, 0))?;
        let style = &theme.header;

        // Current Directory
        let prefix_str = state.target_path.to_string_lossy();
        let suffix_str = state.cur_subdir.to_string_lossy();
        style.label.write_text(&mut w, "Current Directory: ")?;
        style.path_prefix.write_text(&mut w, prefix_str.as_ref())?;
        style.label.write_text(&mut w, "/")?;
        style.path_suffix.write_text(&mut w, suffix_str.as_ref())?;
        style.label.write_text(&mut w, ", ")?;

        // Size
        if state.cur_subdir.to_string_lossy().is_empty() {
            let target_size_str = bytes_to_str(target_dir_size);
            style.label.write_text(&mut w, "Size: ")?;
            style.size_total.write_text(&mut w, &target_size_str)?;
        } else {
            let size_percent = (cur_dir_size as f64 / target_dir_size as f64) * 100.0;
            let target_size_str = bytes_to_str(target_dir_size);
            let cur_dir_size_str = bytes_to_str(cur_dir_size);
            style.label.write_text(&mut w, "Size: ")?;
            style.size_current.write_text(&mut w, &cur_dir_size_str)?;
            style.label.write_text(&mut w, " / ")?;
            style.size_total.write_text(&mut w, &target_size_str)?;
            style.label.write_text(&mut w, " (")?;
            if let Some(s) = &style.size_percent {
                s.write_text(&mut w, &format!("{size_percent:.2}"))?;
            } else {
                let s = style::SetForegroundColor(color_to_crossterm(&percent_to_color(
                    size_percent,
                    &theme,
                )));
                write!(w, "{s}{size_percent:.2}{}", ResetColor)?;
            }
            style.label.write_text(&mut w, ")")?;
        }
        style.label.write_text(&mut w, ", ")?;

        // Sort Mode
        style.label.write_text(&mut w, "Sort by ")?;
        style
            .sort_mode
            .write_text(&mut w, &format!("{:?}", state.sort_by))?;

        // Progress State
        let is_scan_done = match target_dir_state {
            FileState::Queued | FileState::InProgress => {
                style.label.write(&mut w)?;
                write!(w, ", Scanning {}", loading_text(state.frame))?;
                false
            }
            FileState::Finished => true,
            FileState::Failed(err) => {
                style.label.write(&mut w)?;
                write!(
                    w,
                    ", {}ERROR: {}{}",
                    style::SetForegroundColor(rgb(1.0, 0.0, 0.0)),
                    err,
                    style::ResetColor,
                )?;
                true
            }
        };

        // File View Header
        write!(w, "{}", cursor::MoveTo(0, 2))?;
        let style = &theme.file_table;
        if !is_scan_done {
            style.progress_header.write_text(&mut w, "S")?;
            write!(w, "  ")?;
        }
        let name_col_width = i64::max(
            8,
            i64::min(
                32,
                state.term_size[0] as i64 - 46 - if is_scan_done { 0 } else { 3 },
            ),
        );
        style.name_header.write_text(&mut w, "Name")?;
        write!(w, "  ")?;
        for _ in 0..name_col_width - 4 {
            write!(w, " ")?;
        }

        write!(w, "  ")?;
        style.size_abs_header.write_text(&mut w, "Size Abs")?;
        write!(w, "  ")?;

        write!(w, "  ")?;
        style.size_rel_header.write_text(&mut w, "Size Rel")?;
        write!(w, "  ")?;

        if !state.cur_subdir.to_string_lossy().is_empty() {
            write!(w, "  ")?;
            style.size_cur_header.write_text(&mut w, "Size Cur")?;
            write!(w, "  ")?;
        }
        style.modtime_header.write_text(&mut w, "Last Modified")?;

        // File View Rows
        let children = sorted_entries(&file_tree, state);
        let skip = i64::max(
            0,
            state.cur_subdir_entry_ix() as i64 - ((state.term_size[1] as i64 - 3) / 2),
        ) as usize;
        for (j, (i, (name, node, inode, is_dir))) in
            children.iter().enumerate().skip(skip).enumerate()
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
                maybe_selected(&style.progress_value).write_text(&mut w, state_str)?;
                bg_style.write_text(&mut w, "  ")?;
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
            maybe_selected(&style.progress_value).write_text(&mut w, &name)?;
            x += name_col_width;

            // Make sure background color is printed even if name is shorter than max length.
            if is_selected {
                let spaces: String = (name.chars().count()..name_col_width as usize)
                    .map(|_| " ")
                    .collect();
                bg_style.write_text(&mut w, &spaces)?;
            } else {
                write!(w, "{}", cursor::MoveTo(x as u16, j as u16 + 3))?;
            };
            bg_style.write_text(&mut w, "  ")?;

            // Size Abs
            let size_abs_str = bytes_to_str(inode.size_in_bytes);
            let spaces: String = (size_abs_str.len()..10).map(|_| " ").collect();
            bg_style.write_text(&mut w, &spaces)?;
            maybe_selected(&style.size_abs_value).write_text(&mut w, &size_abs_str)?;
            bg_style.write_text(&mut w, "  ")?;

            // Size Rel
            let abs_size_percent = (inode.size_in_bytes as f64 / target_dir_size as f64) * 100.0;
            let abs_size_percent_str = format!("{:6.2} %", abs_size_percent);
            let spaces: String = (abs_size_percent_str.len()..10).map(|_| " ").collect();
            bg_style.write_text(&mut w, &spaces)?;
            if let Some(s) = &style.size_rel_value {
                s.write(&mut w)?;
            } else {
                bg_style
                    .clone()
                    .fg(percent_to_color(abs_size_percent, theme))
                    .write(&mut w)?;
            }
            write!(w, "{}", abs_size_percent_str)?;
            write!(w, "{}", style::ResetColor)?;
            bg_style.write_text(&mut w, "  ")?;

            // Size Cur
            if !state.cur_subdir.to_string_lossy().is_empty() {
                let size_percent = (inode.size_in_bytes as f64 / cur_dir_size as f64) * 100.0;
                let size_percent_str = format!("{:6.2} %", size_percent);
                let spaces: String = (abs_size_percent_str.len()..10).map(|_| " ").collect();
                bg_style.write_text(&mut w, &spaces)?;
                if let Some(s) = &style.size_cur_value {
                    s.write(&mut w)?;
                } else {
                    bg_style
                        .clone()
                        .fg(percent_to_color(abs_size_percent, theme))
                        .write(&mut w)?;
                }
                write!(w, "{}", size_percent_str)?;
                write!(w, "{}", style::ResetColor)?;
                bg_style.write_text(&mut w, "  ")?;
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
                .format("%Y[-]%m[-]%d  %H[:]%M[:]%S")
                .to_string()
                .replace("[", &s_delim)
                .replace("]", &s_val);
            write!(w, "{}", s_val)?;
            write!(w, "{}", last_modified)?;
            write!(w, "{}", style::ResetColor)?;
        }
    }

    state.frame += 1;

    stdout.write_all(&w)?;
    stdout.flush()?;

    Ok(())
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
