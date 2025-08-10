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
    State,
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
) -> Vec<(&'a OsString, &'a FileNode, bool)> {
    let children = &file_tree.get(&state.cur_subdir).unwrap().children;
    let mut children = children
        .into_iter()
        .map(|(name, node)| {
            let path = state.target_path.join(&state.cur_subdir).join(name);
            let is_dir = path.is_dir() && !path.is_symlink();
            (name, node, is_dir)
        })
        .collect::<Vec<_>>();
    children.sort_by_key(|(name, _node, is_dir)| (!*is_dir, *name));
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
        match target_dir_state {
            FileState::Queued | FileState::InProgress => {
                write!(w, ", Scanning {}", loading_text(state.frame))?;
            }
            FileState::Finished => (),
            FileState::Failed(err) => {
                write!(
                    w,
                    ", {}ERROR: {}{}",
                    style::SetForegroundColor(rgb(1.0, 0.0, 0.0)),
                    err,
                    style::ResetColor,
                )?;
            }
        };

        write!(w, "{}", cursor::MoveTo(0, 2))?;
        write!(w, "{}", style::SetAttribute(Attribute::Bold))?;
        write!(w, "{:<15}", "Name")?;
        write!(w, "{:<10}", "Size")?;
        write!(w, "{:<10}", "Size Rel")?;
        write!(w, "{:<10}", "Size Abs")?;
        write!(w, "{:<20}", "Modification")?;
        write!(w, "{:<6}", "Type")?;
        write!(w, "{:<10}", "State")?;
        write!(w, "{}", style::ResetColor)?;

        let children = sorted_entries(&file_tree, &state);
        let skip = i64::max(
            0,
            state.cur_subdir_entry_ix() as i64 - state.term_size[1] as i64 / 2,
        ) as usize;
        for (j, (i, (name, node, _is_dir))) in children.iter().enumerate().skip(skip).enumerate() {
            write!(w, "{}", cursor::MoveTo(0, j as u16 + 3))?;
            if i == state.cur_subdir_entry_ix() {
                write!(w, "{}", style::SetForegroundColor(rgb(0.4, 1.0, 0.5)))?;
            }
            let name = name.to_string_lossy().chars().take(13).collect::<String>();
            write!(w, "{:<15}", name)?;
            if i == state.cur_subdir_entry_ix() {
                write!(w, "{}", style::ResetColor)?;
            }
            let inode = file_tree.inodes.get(&node.inode_id).unwrap();
            write!(w, "{:>10}", bytes_to_str(inode.size_in_bytes))?;
            let size_percent = (inode.size_in_bytes as f64 / cur_dir_size as f64) * 100.0;
            write!(
                w,
                "{}",
                style::SetForegroundColor(percent_to_color(size_percent))
            )?;
            write!(w, "{:<10}", format!("{:6.2} %", size_percent))?;
            write!(w, "{}", style::ResetColor)?;
            let abs_size_percent = (inode.size_in_bytes as f64 / target_dir_size as f64) * 100.0;
            write!(
                w,
                "{}",
                style::SetForegroundColor(percent_to_color(abs_size_percent))
            )?;
            write!(w, "{:<10}", format!("{:6.2} %", abs_size_percent))?;
            write!(w, "{}", style::ResetColor)?;
            let last_modified = DateTime::<Local>::from(inode.last_modified);
            let last_modified = last_modified.format("%Y-%m-%d %H:%M:%S");
            write!(w, "{:<20}", last_modified)?;
            let is_dir = if inode.is_dir { "D" } else { "F" };
            write!(w, "{:<6}", is_dir)?;
            write!(w, "{:<10?}", node.state)?;
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
