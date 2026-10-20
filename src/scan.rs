use std::{
    collections::HashSet,
    fs::symlink_metadata,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::Result;

/// A filesystem inode, uniquely identified by the (device, inode number)
/// pair. The inode number alone is only unique within one filesystem, so it
/// must not be used as an identity on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InodeId {
    pub dev: u64,
    pub ino: u64,
}

impl InodeId {
    pub fn new(meta: &std::fs::Metadata) -> Self {
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    /// Disk usage of the entry in bytes, i.e. its number of allocated blocks (`st_blocks * 512`),
    /// as used by `du`. For a directory this is the space of its own filesystem entry, not of its
    /// contents. For sparse files the size of the actually allocated blocks is used instead of
    /// their apparent size.
    pub allocated_bytes: u64,
    /// The last modification time of the entry, or `None` if it could not be read
    /// (e.g. on some network or FUSE mounts).
    pub last_modified: Option<SystemTime>,
    pub num_hardlinks: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    StartedScan(PathBuf, InodeId, Inode),
    FinishedScan(PathBuf, InodeId),
    Error(PathBuf, String),
}

impl ScanEvent {
    pub fn map_path_bufs(&mut self, f: impl FnOnce(&mut PathBuf)) {
        match self {
            ScanEvent::StartedScan(path, _, _) => f(path),
            ScanEvent::FinishedScan(path, _) => f(path),
            ScanEvent::Error(path, _) => f(path),
        }
    }
}

/// Recursively scan all filesystem entries of `dir` and emit events via
/// `on_event`. This function returns `Err` only if `on_event` returns `Err`.
///
/// `is_deleted` is consulted before descending into a path; deleted paths
/// are neither scanned nor reported. This lets the caller stop the walk in
/// subtrees that were removed while the scan was in progress.
pub fn scan(
    dir: impl AsRef<Path>,
    mut on_event: impl FnMut(ScanEvent) -> Result<()>,
    is_deleted: impl Fn(&Path) -> bool,
) -> Result<()> {
    let mut visited_dirs = HashSet::new();
    scan_(dir.as_ref(), &mut on_event, &is_deleted, &mut visited_dirs)?;
    Ok(())
}

fn scan_(
    path: &Path,
    on_event: &mut impl FnMut(ScanEvent) -> Result<()>,
    is_deleted: &impl Fn(&Path) -> bool,
    visited_dirs: &mut HashSet<InodeId>,
) -> Result<()> {
    if is_deleted(path) {
        return Ok(());
    }

    let meta = match symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) => {
            on_event(ScanEvent::Error(
                path.to_path_buf(),
                format!("Cannot read metadata:\n{e}"),
            ))?;
            return Ok(());
        }
    };

    let last_modified = match meta.modified() {
        Ok(t) => Some(t),
        Err(e) => {
            on_event(ScanEvent::Error(
                path.to_path_buf(),
                format!("Cannot read modtime:\n{e}"),
            ))?;
            None
        }
    };

    let inode_id = InodeId::new(&meta);
    let is_dir = meta.is_dir();
    on_event(ScanEvent::StartedScan(
        path.to_path_buf(),
        inode_id,
        Inode {
            allocated_bytes: meta.blocks() * 512,
            last_modified,
            num_hardlinks: meta.nlink(),
            is_dir,
        },
    ))?;

    if is_dir {
        // A directory inode that was already walked (e.g. reached again via
        // a bind mount) is not walked again; the first path "owns" the
        // inode. This also prevents infinite recursion.
        if !visited_dirs.insert(inode_id) {
            on_event(ScanEvent::FinishedScan(path.to_path_buf(), inode_id))?;
            return Ok(());
        }

        let iter = match path.read_dir() {
            Ok(iter) => iter,
            Err(e) => {
                on_event(ScanEvent::Error(
                    path.to_path_buf(),
                    format!("Cannot read directory contents:\n{e}"),
                ))?;
                return Ok(());
            }
        };
        for e in iter {
            let e = match e {
                Ok(e) => e,
                Err(e) => {
                    on_event(ScanEvent::Error(
                        path.to_path_buf(),
                        format!("Cannot read directory entry:\n{e}"),
                    ))?;
                    continue;
                }
            };
            scan_(&e.path(), on_event, is_deleted, visited_dirs)?;
            if is_deleted(path) {
                // The directory was deleted while it was being walked.
                break;
            }
        }
    }

    on_event(ScanEvent::FinishedScan(path.to_path_buf(), inode_id))?;

    Ok(())
}
