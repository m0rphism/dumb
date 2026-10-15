use std::{
    collections::HashSet,
    fs::symlink_metadata,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};

pub type InodeId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    pub size_in_bytes: u64,
    pub last_modified: SystemTime,
    pub num_hardlinks: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathType {
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    StartedScan(PathBuf, InodeId, PathType),
    NewInode(InodeId, Inode),
    FinishedScan(PathBuf, InodeId),
    FailedScan(PathBuf, String),
}

impl ScanEvent {
    pub fn map_path_bufs(&mut self, f: impl FnOnce(&mut PathBuf)) {
        match self {
            ScanEvent::StartedScan(path, _, _) => f(path),
            ScanEvent::NewInode(_, _) => (),
            ScanEvent::FinishedScan(path, _) => f(path),
            ScanEvent::FailedScan(path, _) => f(path),
        }
    }
}

/// Scan all filesystem entries of `dir` transitively and emit events via the `on_event` handler.
/// This function returns `Err` only if `on_event` returns `Err`.
pub fn scan(
    dir: impl AsRef<Path>,
    mut on_event: impl FnMut(ScanEvent) -> Result<()>,
) -> Result<()> {
    let mut visited_inodes = HashSet::new();
    let _ = scan_(dir.as_ref(), &mut on_event, &mut visited_inodes)?;
    Ok(())
}

fn scan_(
    path: &Path,
    on_event: &mut impl FnMut(ScanEvent) -> Result<()>,
    visited_inodes: &mut HashSet<InodeId>,
) -> Result<(u64, SystemTime)> {
    let meta = match symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) => {
            on_event(ScanEvent::FailedScan(
                path.to_path_buf(),
                format!("Cannot read metadata of path '{path:?}':\n{e}"),
            ))?;
            return Ok((0, SystemTime::UNIX_EPOCH));
        }
    };

    let mut last_modified = match meta.modified() {
        Ok(t) => t,
        Err(e) => {
            on_event(ScanEvent::FailedScan(
                path.to_path_buf(),
                format!("Cannot read modtime of path '{path:?}' (using UNIX_EPOCH instead):\n{e}"),
            ))?;
            SystemTime::UNIX_EPOCH
        }
    };

    let inode_id = meta.ino();
    on_event(ScanEvent::StartedScan(
        path.to_path_buf(),
        inode_id,
        if meta.is_dir() {
            PathType::Dir
        } else {
            PathType::File
        },
    ))?;

    let size_in_bytes = if meta.is_dir() && !meta.is_symlink() {
        let iter = match path.read_dir() {
            Ok(iter) => iter,
            Err(e) => {
                on_event(ScanEvent::FailedScan(
                    path.to_path_buf(),
                    format!("Cannot read directory contents of '{path:?}':\n{e}"),
                ))?;
                return Ok((0, SystemTime::UNIX_EPOCH));
            }
        };
        let mut size_in_bytes = 0;
        for e in iter {
            let e = match e {
                Ok(e) => e,
                Err(_e) => {
                    // TODO
                    continue;
                }
            };
            let (s, m) = scan_(&e.path(), on_event, visited_inodes)?;
            size_in_bytes += s;
            last_modified = SystemTime::max(last_modified, m);
        }
        on_event(ScanEvent::NewInode(
            inode_id,
            Inode {
                size_in_bytes,
                last_modified,
                num_hardlinks: 1,
                is_dir: true,
            },
        ))?;
        size_in_bytes
    } else {
        if visited_inodes.insert(inode_id) {
            let size_in_bytes = meta.size();
            on_event(ScanEvent::NewInode(
                inode_id,
                Inode {
                    size_in_bytes,
                    last_modified,
                    num_hardlinks: meta.nlink(),
                    is_dir: false,
                },
            ))?;
            size_in_bytes
        } else {
            0
        }
    };

    on_event(ScanEvent::FinishedScan(path.to_path_buf(), inode_id))?;

    Ok((size_in_bytes, last_modified))
}
