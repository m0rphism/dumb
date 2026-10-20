use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

use anyhow::Result;

use crate::{
    SortBy,
    config::SizeMetric,
    file_tree::{FileNode, FileState, FileTree},
    scan::Inode,
};

/// An entry of a directory in the file view.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// The file name of the entry.
    pub name: Arc<OsString>,
    /// The state of the entry's node in the file tree (relevant for the progress column).
    pub node_state: FileState,
    /// The number of errors in the entry's subtree (relevant for the error row highlighting).
    pub num_child_errors: usize,
    /// The inode of the entry.
    pub inode: Inode,
    /// Whether the entry is a real directory (not a symlink).
    pub is_dir: bool,
    /// The size of the entry in bytes according to the active size metric.
    pub size: u64,
}

#[derive(Debug)]
pub struct RenderInfo {
    pub cur_dir_size: u64,
    pub root_dir_size: u64,
    pub root_dir_state: FileState,
    pub num_errors: usize,
}

/// The view parameters that determine how the entries of a directory are
/// snapshotted and sorted. These are part of [`ViewSnapshot`] and are compared
/// against the current view to detect a stale snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewParams {
    /// The viewed subdirectory, relative to the root directory.
    pub subdir: PathBuf,
    /// How the entries are sorted.
    pub sort_by: SortBy,
    /// Whether to sort directories before files (only relevant for [`SortBy::Name`]).
    pub sort_dirs_before_files: bool,
    /// The size metric used to display and sort the entries.
    pub size_metric: SizeMetric,
}

/// A consistent, versioned snapshot of the file tree for the currently viewed
/// subdirectory.
///
/// This is the single source of truth for both file tree rendering and user actions: the
/// header info, the sorted entry list, and the error count all come from the
/// same tree version, so a rendered frame is internally consistent and every
/// action refers to the same data that is on screen.
#[derive(Debug)]
pub struct Snapshot {
    /// Whether the snapshot has been populated at least once.
    pub built: bool,
    /// The file tree version the snapshot was captured at.
    pub version: u64,
    /// The view parameters the snapshot was built for.
    pub params: ViewParams,
    /// The sorted entries of the subdirectory sepcified in [`ViewSnapshot::params`].
    pub entries: Vec<FileEntry>,
    /// The cached header info for rendering.
    pub header: RenderInfo,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            built: false,
            version: 0,
            params: ViewParams {
                subdir: PathBuf::new(),
                sort_by: SortBy::Size,
                sort_dirs_before_files: false,
                size_metric: SizeMetric::Freed,
            },
            entries: vec![],
            header: RenderInfo {
                cur_dir_size: 0,
                root_dir_size: 0,
                root_dir_state: FileState::Queued,
                num_errors: 0,
            },
        }
    }
}

impl Snapshot {
    /// Rebuild the snapshot synchronously for `params`.
    ///
    /// Used on startup, on changes to the current directory, and after a deletion, i.e. exactly when
    /// the user expects the displayed list to change immediately.
    ///
    /// This is the only place where the main thread takes the file tree lock.
    pub fn sync_rebuild(
        &mut self,
        params: &ViewParams,
        file_tree: &Mutex<FileTree>,
        file_tree_version: &AtomicU64,
    ) {
        *self = {
            let file_tree = file_tree.lock().unwrap();
            match Self::capture(&file_tree, file_tree_version, params) {
                Some(snapshot) => snapshot,
                // The current directory no longer exists (potentially deleted outside of dumb), so set
                // an empty snapshot for it.
                None => Snapshot {
                    built: true,
                    version: file_tree_version.load(Ordering::Relaxed),
                    params: params.clone(),
                    entries: vec![],
                    header: RenderInfo {
                        cur_dir_size: 0,
                        root_dir_size: match params.size_metric {
                            SizeMetric::Freed => file_tree.root.freed_size,
                            SizeMetric::Naive => file_tree.root.naive_size,
                        },
                        root_dir_state: file_tree.root.state.clone(),
                        num_errors: file_tree.errors.len(),
                    },
                },
            }
        };
        self.sort_entries(params);
    }

    /// Spawn the snapshot builder thread, which takes build requests in form of [`ViewParams`] and calls
    /// `on_built` on the resulting snapshots.
    ///
    /// Builds are requested periodically during scanning, and by changes to sort order or size metric.
    ///
    /// Deletions and changes to the current directory do not go through this path, and instead use
    /// [`Self::sync_rebuild`].
    ///
    /// The purpose of this thread is to keep the expensive snapshot+sort work off the main thread,
    /// which would otherwise hurt TUI responsiveness when viewing a large directory while it is
    /// being scanned.
    pub fn spawn_snapshot_builder_thread(
        file_tree: Arc<Mutex<FileTree>>,
        tree_version: Arc<AtomicU64>,
        request_receiver: mpsc::Receiver<ViewParams>,
        mut on_built: impl FnMut(Snapshot) -> Result<()> + Send + 'static,
    ) -> JoinHandle<()> {
        std::thread::spawn(move || {
            while let Ok(mut params) = request_receiver.recv() {
                // Only the most recent request is relevant.
                while let Ok(newer) = request_receiver.try_recv() {
                    params = newer;
                }

                // Take a consistent snapshot under the tree lock. If the subdirectory no longer
                // exists (e.g. it was deleted after the request was made), drop the request.
                let mut snapshot = {
                    let file_tree = file_tree.lock().unwrap();
                    match Self::capture(&file_tree, &tree_version, &params) {
                        Some(snapshot) => snapshot,
                        None => continue,
                    }
                };

                snapshot.sort_entries(&params);

                if on_built(snapshot).is_err() {
                    break;
                }
            }
        })
    }

    /// Sorts an owned snapshot of entries according to `params`.
    fn sort_entries(&mut self, params: &ViewParams) {
        match params.sort_by {
            SortBy::Name => {
                self.entries.sort_by(|e1, e2| {
                    let is_file1 = !e1.is_dir && params.sort_dirs_before_files;
                    let is_file2 = !e2.is_dir && params.sort_dirs_before_files;
                    is_file1
                        .cmp(&is_file2)
                        .then_with(|| e1.name.as_ref().cmp(e2.name.as_ref()))
                });
            }
            SortBy::Size => {
                self.entries.sort_by(|e1, e2| {
                    (e2.size, e1.name.as_ref()).cmp(&(e1.size, e2.name.as_ref()))
                });
            }
            SortBy::ModTime => {
                self.entries.sort_by(|e1, e2| {
                    (e2.inode.last_modified, e1.name.as_ref())
                        .cmp(&(e1.inode.last_modified, e2.name.as_ref()))
                });
            }
        }
    }

    /// Captures a consistent [`ViewSnapshot`] of the file tree for `params`.
    ///
    /// Returns `None` if the subdirectory in `params` does not exist. The returned entries are
    /// unsorted; run [`sort_entries`] on them before publishing the snapshot.
    fn capture(
        file_tree: &FileTree,
        file_tree_version: &AtomicU64,
        params: &ViewParams,
    ) -> Option<Self> {
        let size_of = |n: &FileNode| match params.size_metric {
            SizeMetric::Freed => n.freed_size,
            SizeMetric::Naive => n.naive_size,
        };
        let cur_node = file_tree.get(&params.subdir)?;
        let entries = cur_node
            .children
            .iter()
            .map(|(name, node)| {
                let inode = node.inode;
                let size = match params.size_metric {
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
                FileEntry {
                    name: name.clone(),
                    node_state: node.state.clone(),
                    num_child_errors: node.num_child_errors,
                    inode,
                    is_dir: inode.is_dir,
                    size,
                }
            })
            .collect();
        let header = RenderInfo {
            cur_dir_size: size_of(cur_node),
            root_dir_size: size_of(&file_tree.root),
            root_dir_state: file_tree.root.state.clone(),
            num_errors: file_tree.errors.len(),
        };
        Some(Self {
            built: true,
            version: file_tree_version.load(Ordering::Relaxed),
            params: params.clone(),
            entries,
            header,
        })
    }
}
