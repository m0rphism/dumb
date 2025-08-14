use std::{
    ffi::OsString,
    io::{Stdout, Write},
    path::{Path, PathBuf},
};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::{
    cursor,
    style::{self, Attribute, Color},
    terminal::{Clear, ClearType},
};

use crate::{
    file_tree::{self, FileNode, FileState, FileTree},
    scan::Inode,
    SortBy, State,
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
        .into_iter()
        .map(|(name, node)| {
            let path = state.target_path.join(&state.cur_subdir).join(name);
            let is_dir = path.is_dir() && !path.is_symlink();
            let inode = file_tree.inodes.get(&node.inode_id).unwrap().clone();
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
            children.sort_by_key(|(_name, _node, inode, is_dir)| {
                // let is_file = !*is_dir && state.sort_dirs_before_files;
                // (is_file, inode.size_in_bytes)
                inode.size_in_bytes
            });
            children.reverse();
        }
        SortBy::ModTime => {
            children.sort_by_key(|(_name, _node, inode, is_dir)| {
                // let is_file = !*is_dir && state.sort_dirs_before_files;
                // (is_file, inode.last_modified)
                inode.last_modified
            });
            children.reverse();
        }
    }
    children
}

pub fn render_state(stdout: &mut Stdout, state: &mut State) -> Result<()> {
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
        write!(
            w,
            "Current Directory: {}{}{}{}/{}{}, ",
            style::SetForegroundColor(rgb(0.4, 0.5, 1.0)),
            state.target_path.to_string_lossy(),
            style::ResetColor,
            style::SetForegroundColor(rgb(1.0, 0.4, 0.5)),
            state.cur_subdir.to_string_lossy(),
            style::ResetColor,
        )?;
        if state.cur_subdir.to_string_lossy().is_empty() {
            write!(
                w,
                "Size: {}{}{}",
                style::SetForegroundColor(rgb(0.4, 0.5, 1.0)),
                bytes_to_str(target_dir_size),
                style::ResetColor,
            )?;
        } else {
            let size_percent = (cur_dir_size as f64 / target_dir_size as f64) * 100.0;
            write!(
                w,
                "Size: {}{}{} / {}{}{} ({}{}{})",
                style::SetForegroundColor(rgb(1.0, 0.4, 0.5)),
                bytes_to_str(cur_dir_size),
                style::ResetColor,
                style::SetForegroundColor(rgb(0.4, 0.5, 1.0)),
                bytes_to_str(target_dir_size),
                style::ResetColor,
                // style::SetForegroundColor(rgb(0.4, 1.0, 0.5)),
                style::SetForegroundColor(percent_to_color(size_percent)),
                format!("{:.2} %", size_percent),
                style::ResetColor,
            )?;
        }

        write!(
            w,
            ", Sort by {}{:?}{}",
            style::SetForegroundColor(rgb(0.4, 0.5, 1.0)),
            state.sort_by,
            style::ResetColor,
        )?;

        let is_scan_done = match target_dir_state {
            FileState::Queued | FileState::InProgress => {
                write!(w, ", Scanning {}", loading_text(state.frame))?;
                false
            }
            FileState::Finished => true,
            FileState::Failed(err) => {
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

        write!(w, "{}", cursor::MoveTo(0, 2))?;
        write!(w, "{}", style::SetAttribute(Attribute::Bold))?;
        if !is_scan_done {
            write!(w, "{:<1}  ", "S")?;
        }
        let name_col_width = i64::max(
            8,
            i64::min(
                32,
                state.term_size[0] as i64 - 46 - if is_scan_done { 0 } else { 3 },
            ),
        );
        write!(w, "{}  ", "Name")?;
        for _ in 0..name_col_width - 4 {
            write!(w, " ")?;
        }
        write!(w, "{:<10}  ", "Size Abs")?;
        write!(w, "{:<10}  ", "Size Rel")?;
        if !state.cur_subdir.to_string_lossy().is_empty() {
            write!(w, "{:<10}  ", "Size Cur")?;
        }
        write!(w, "{:<20}", "Last Modified")?;
        write!(w, "{}", style::ResetColor)?;

        let children = sorted_entries(&file_tree, &state);
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

            if is_selected {
                // write!(w, "{}", style::SetForegroundColor(rgb(0.4, 1.0, 0.5)))?;
                write!(w, "{}", style::SetBackgroundColor(rgb(0.1, 0.1, 0.1)))?;
            }

            if !is_scan_done {
                let state_str = match node.state {
                    FileState::Queued | FileState::InProgress => "…",
                    FileState::Finished => " ",
                    FileState::Failed(_) => "E",
                };
                write!(w, "{:<1}  ", state_str)?;
                x += 3;
            }

            let mut name = name
                .to_string_lossy()
                .chars()
                .take(name_col_width as usize)
                .collect::<String>();
            if *is_dir {
                name += "/";
            }
            if inode.is_dir {
                write!(w, "{}", style::SetForegroundColor(rgb(0.7, 0.7, 0.7)))?;
            }
            write!(w, "{name}  ")?;
            x += name_col_width;

            // Make sure background color is printed even if name is shorter than max length.
            if is_selected {
                for _ in name.chars().count()..name_col_width as usize {
                    write!(w, " ")?;
                }
            }
            write!(w, "{}  ", cursor::MoveTo(x as u16, j as u16 + 3))?;

            write!(w, "{}", style::SetForegroundColor(style::Color::White))?;
            write!(w, "{:>10}  ", bytes_to_str(inode.size_in_bytes))?;

            let abs_size_percent = (inode.size_in_bytes as f64 / target_dir_size as f64) * 100.0;
            write!(
                w,
                "{}",
                style::SetForegroundColor(percent_to_color(abs_size_percent))
            )?;
            write!(w, "{:<10}  ", format!("{:6.2} %", abs_size_percent))?;
            write!(w, "{}", style::SetForegroundColor(style::Color::White))?;

            if !state.cur_subdir.to_string_lossy().is_empty() {
                let size_percent = (inode.size_in_bytes as f64 / cur_dir_size as f64) * 100.0;
                write!(
                    w,
                    "{}",
                    style::SetForegroundColor(percent_to_color(size_percent))
                )?;
                write!(w, "{:<10}  ", format!("{:6.2} %", size_percent))?;
                write!(w, "{}", style::SetForegroundColor(style::Color::White))?;
            }

            let last_modified = DateTime::<Local>::from(inode.last_modified);
            let gray = format!("{}", style::SetForegroundColor(rgb(0.5, 0.5, 0.5)));
            // let gray = format!("{}", style::SetForegroundColor(style::Color::White));
            let white = format!("{}", style::SetForegroundColor(style::Color::White));
            let last_modified = last_modified
                .format("%Y[-]%m[-]%d  %H[:]%M[:]%S")
                .to_string()
                .replace("[", &gray)
                .replace("]", &white);
            write!(w, "{:<20}", last_modified)?;
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

pub fn percent_to_color(f: f64) -> Color {
    let f = (f / 100.0) as f32;
    let f = f.sqrt();
    rgb(0.2 + f * 0.8, 0.2, 0.2)
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
