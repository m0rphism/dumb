use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use crate::scan::{Inode, InodeId, ScanEvent};

/// In-memory representation of a scanned directory tree.
///
/// Size bookkeeping invariants:
/// - All sizes are disk usage in bytes (allocated blocks, as in `du`), not apparent file sizes.
/// - For every directory, `naive_size` is the sum of the disk usage of all entries in its subtree
///   (multiple hardlinks to the same content are counted multiple times), and `freed_size` is the
///   space that would be freed by recursively deleting the directory.
/// - To properly account for hardlinks, every file inode contributes at most once, at the deepest
///   common ancestor of all of its paths.
///   When a name is deleted, the contribution moves down to the lowest common ancestor of the
///   remaining paths (no contribution if that ancestor is outside of the root dir), or disappears
///   if the inode itself is deleted.
#[derive(Debug, Clone)]
pub struct FileTree {
    /// The root node of the directory tree
    pub root: FileNode,
    /// Information about the inodes behind the directory tree
    pub inodes: HashMap<InodeId, Inode>,
    /// Errors collected during scanning, e.g. when a directory could not be read due to missing
    /// permissions.
    pub errors: Vec<(PathBuf, String)>,
    /// Relative paths of deleted subtrees. Scan events with paths under any of these are dropped.
    /// This ensures correct behavior if the user deletes an entry in the TUI while it is being
    /// scanned.
    pub deleted_paths: Vec<PathBuf>,
    /// Maps inodes with multiple paths (hardlinks) to their paths and place of contribution. Kept
    /// for the whole session so that on deletions of one path the size contributions of the other
    /// paths can be adjusted without having to retraverse the entire root directory.
    pub linked: HashMap<InodeId, LinkRecord>,
}

/// Information about an inode with multiple paths (hardlinks).
#[derive(Debug, Clone)]
pub struct LinkRecord {
    /// The paths of the inode
    pub paths: Vec<PathBuf>,
    /// Where the size of this inode currently contributes. This is `None` until all names have been
    /// scanned.
    pub contributes_at: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct FileNode {
    pub children: HashMap<OsString, FileNode>,
    pub inode_id: InodeId,
    pub state: FileState,
    pub num_child_errors: usize,
    /// Naive size (see [`FileTree`]).
    pub naive_size: u64,
    /// Freed size (see [`FileTree`]).
    pub freed_size: u64,
}

#[derive(Debug, Clone)]
pub enum FileState {
    Queued,
    Finished,
    Failed(String),
}

impl FileNode {
    pub fn new(inode_id: InodeId) -> Self {
        Self {
            children: HashMap::new(),
            state: FileState::Queued,
            inode_id,
            num_child_errors: 0,
            naive_size: 0,
            freed_size: 0,
        }
    }

    pub fn get(&self, path: &Path) -> Option<&FileNode> {
        let mut node = self;
        for c in path.components() {
            let c = &c.as_os_str().to_os_string();
            node = node.children.get(c)?;
        }
        Some(node)
    }

    pub fn get_mut(&mut self, path: &Path) -> Option<&mut FileNode> {
        let mut node = self;
        for c in path.components() {
            let c = &c.as_os_str().to_os_string();
            node = node.children.get_mut(c)?;
        }
        Some(node)
    }

    /// Create the node at `path` (relative to the root).
    /// All intermediate nodes must already exist, which holds because the scan is pre-order and
    /// deleted paths produce no events.
    pub fn set_queued(&mut self, path: &Path, inode_id: InodeId) {
        let mut node = self;
        let cs = path.components().collect::<Vec<_>>();
        if !cs.is_empty() {
            for c in cs.iter().take(cs.len() - 1) {
                let c = &c.as_os_str().to_os_string();
                node = node.children.get_mut(c).unwrap()
            }
            let c = cs.last().unwrap().as_os_str().to_os_string();
            node.children.insert(c, FileNode::new(inode_id));
        } else {
            node.inode_id = inode_id;
        }
    }

    /// Mark the node at `path` as failed and increment `num_child_errors`
    /// on all of its existing proper ancestors.
    /// All intermediate nodes must already exist, which holds because the scan is pre-order and
    /// deleted paths produce no events.
    pub fn set_error(&mut self, path: &Path, error: String) {
        let mut node = self;
        for c in path.components() {
            node.num_child_errors += 1;
            match node.children.get_mut(&c.as_os_str().to_os_string()) {
                Some(child) => node = child,
                None => return,
            }
        }
        node.state = FileState::Failed(error);
    }

    fn modify_along(&mut self, dir_path: &Path, mut f: impl FnMut(&mut FileNode)) {
        let mut node = self;
        for c in dir_path.components() {
            f(node);
            let c = c.as_os_str().to_os_string();
            match node.children.get_mut(&c) {
                Some(child) => node = child,
                None => return,
            }
        }
        f(node);
    }

    /// Add `naive`/`freed` to every node on the root..dir_path chain, both
    /// endpoints included. Stops at the first missing node.
    pub fn add_along(&mut self, dir_path: &Path, naive: u64, freed: u64) {
        self.modify_along(dir_path, |node| {
            node.naive_size += naive;
            node.freed_size += freed;
        });
    }

    /// Saturating subtraction counterpart of [`FileNode::add_along`].
    pub fn sub_along(&mut self, dir_path: &Path, naive: u64, freed: u64) {
        self.modify_along(dir_path, |node| {
            node.naive_size = node.naive_size.saturating_sub(naive);
            node.freed_size = node.freed_size.saturating_sub(freed);
        });
    }
}

impl FileTree {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let meta = path.as_ref().metadata().unwrap();
        Self {
            inodes: HashMap::new(),
            root: FileNode::new(InodeId::new(&meta)),
            errors: vec![],
            deleted_paths: vec![],
            linked: HashMap::new(),
        }
    }

    pub fn get(&self, path: &Path) -> Option<&FileNode> {
        self.root.get(path)
    }

    pub fn get_mut(&mut self, path: &Path) -> Option<&mut FileNode> {
        self.root.get_mut(path)
    }

    /// Returns true if a deleted path is a prefix of `path` or equal to `path`.
    pub fn is_deleted_path(&self, path: &Path) -> bool {
        self.deleted_paths
            .iter()
            .any(|d| path == d.as_path() || path.starts_with(d.as_path()))
    }

    /// Single entry point for scan events.
    ///
    /// - Events with paths under a deleted path are dropped.
    /// - `StartedScan` of a directory adds the disk usage of its own filesystem entry to both sizes
    ///   of itself and of every ancestor (counted once per inode).
    /// - `StartedScan` of a file adds its size to the `naive_size` of every ancestor, and to the
    ///   `freed_size` of the ancestors as well if the inode has exactly one path (contributes to
    ///   the parent). Inodes with multiple paths are tracked in `linked` and contribute to their
    ///   lowest common ancestor directory once all its hardlinked paths have been scanned.
    /// - `FinishedScan` only changes the node state to finished (relevant for TUI).
    pub fn handle_scan_event(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::StartedScan(path, inode_id, inode) => {
                if self.is_deleted_path(&path) {
                    return;
                }
                self.root.set_queued(&path, inode_id);
                let fresh = !self.inodes.contains_key(&inode_id);
                self.inodes.entry(inode_id).or_insert(inode);
                if inode.is_dir {
                    // The size of a directory entry itself is counted once per inode: a directory
                    // reached a second time (e.g. via a bind mount) is owned by the first path and
                    // contributes nothing.
                    if fresh {
                        self.root
                            .add_along(&path, inode.allocated_bytes, inode.allocated_bytes);
                    }
                } else {
                    let parent = path.parent().map(PathBuf::from).unwrap_or_default();
                    // For the naive metric, every path to an inode counts its full size on all the
                    // path's ancestors.
                    self.root.add_along(&parent, inode.allocated_bytes, 0);
                    // For the freed metric, it depends on the amount of paths to the inode.
                    if inode.num_hardlinks == 1 {
                        // A single-path inode contributes to its parent.
                        self.root.add_along(&parent, 0, inode.allocated_bytes);
                    } else {
                        // For a multi-path inode, the path is recorded and, once all paths have
                        // been scanned it contributes to the least common ancestor of the paths.
                        let rec = self.linked.entry(inode_id).or_insert(LinkRecord {
                            paths: vec![],
                            contributes_at: None,
                        });
                        rec.paths.push(path.clone());
                        if rec.paths.len() == inode.num_hardlinks as usize {
                            let r = lca(&rec.paths);
                            self.root.add_along(&r, 0, inode.allocated_bytes);
                            rec.contributes_at = Some(r);
                        }
                    }
                }
            }
            ScanEvent::FinishedScan(path, _inode_id) => {
                if self.is_deleted_path(&path) {
                    return;
                }
                if let Some(node) = self.root.get_mut(&path) {
                    node.state = FileState::Finished;
                }
            }
            ScanEvent::Error(path, err) => {
                if self.is_deleted_path(&path) {
                    return;
                }
                self.root.set_error(&path, err.clone());
                self.errors.push((path, err));
            }
        }
    }

    /// Delete the subtree at `path` (relative to the root directory) from the tree and update all
    /// size bookkeeping if necessary (deleted hardlinks may influence the freed size metric of
    /// other links to the same inode).
    ///
    /// `stat_nlink` is queried with a root-dir-relative path to refresh the link count of a surviving
    /// path of an incomplete multi-path inode. It should return `None` if the path cannot be
    /// resolved.
    ///
    /// Returns `None` if `path` is the root or does not exist in the tree.
    pub fn delete_path(
        &mut self,
        path: &Path,
        stat_nlink: &impl Fn(&Path) -> Option<u64>,
    ) -> Option<()> {
        if path.as_os_str().is_empty() {
            return None;
        }
        let node = self.root.get(path)?;
        let inode_id = node.inode_id;
        let (naive_size, freed_size) = (node.naive_size, node.freed_size);
        let inode = *self.inodes.get(&inode_id)?;
        let parent = path.parent().map(PathBuf::from).unwrap_or_default();

        // Reverse the effects that previous `set_error` calls on in the deleted sub tree had on its
        // ancestors, i.e. decrease their error counters and drop the errors belonging to the
        // removed subtree.
        let removed_errors: Vec<PathBuf> = self
            .errors
            .iter()
            .filter(|(p, _)| p == path || p.starts_with(path))
            .map(|(p, _)| p.clone())
            .collect();
        for p in &removed_errors {
            self.decrement_error_counts(p);
        }
        self.errors
            .retain(|(p, _)| p != path && !p.starts_with(path));

        self.remove_subtree(path)?;

        if inode.is_dir {
            self.root.sub_along(&parent, naive_size, freed_size);
        } else {
            // A deleted file entry only contributed to its parents directly if it is the only path
            // to its inode. File entries with multi-path inodes are handled in the LinkRecord
            // traversal below.
            let freed_delta = if self.linked.contains_key(&inode_id) {
                0
            } else {
                inode.allocated_bytes
            };
            self.root
                .sub_along(&parent, inode.allocated_bytes, freed_delta);
        }

        // Collect multi-path inodes that have at least one path with a deleted prefix.
        let affected: Vec<InodeId> = self
            .linked
            .iter()
            .filter_map(|(inode_id, link_record)| {
                let inode_has_path_with_deleted_prefix = link_record
                    .paths
                    .iter()
                    .any(|n| n == path || n.starts_with(path));
                inode_has_path_with_deleted_prefix.then_some(*inode_id)
            })
            .collect();
        // Update the size contribution of the affected inodes.
        for inode_id in affected {
            let size = self
                .inodes
                .get(&inode_id)
                .map(|i| i.allocated_bytes)
                .unwrap_or(0);
            let Some(link_record) = self.linked.get_mut(&inode_id) else {
                continue;
            };
            link_record
                .paths
                .retain(|n| n != path && !n.starts_with(path));
            if link_record.paths.is_empty() {
                // The inode is gone. If it contributed to an ancestor above the deleted path,
                // then remove it there.
                if let Some(r) = &link_record.contributes_at
                    && !(r == path || r.starts_with(path))
                {
                    self.root.sub_along(r, 0, size);
                }
                self.linked.remove(&inode_id);
            } else if let Some(c) = link_record.contributes_at.clone() {
                let c_new = lca(&link_record.paths);
                if c_new != c {
                    // The contribution of the inode at `c` moves into the subdirectory `c_new`.
                    // Consequently, the freed size of all nodes in between `c` and `c_new` needs to
                    // be increased.
                    self.root.add_along(&c_new, 0, size);
                    self.root.sub_along(&c, 0, size);
                    link_record.contributes_at = Some(c_new);
                }
            } else if let Some(name) = link_record.paths.first().cloned() {
                // Incomplete record: check whether all of the inode's current links are scanned now
                // and, if so, apply the contribution.
                if let Some(n) = stat_nlink(&name)
                    && n as usize == link_record.paths.len()
                {
                    let c = lca(&link_record.paths);
                    self.root.add_along(&c, 0, size);
                    link_record.contributes_at = Some(c);
                }
            }
        }

        self.deleted_paths.push(path.to_path_buf());

        Some(())
    }

    /// Remove the subtree at `path`. Path must be non-empty and relative to the file tree root.
    fn remove_subtree(&mut self, path: &Path) -> Option<()> {
        let comps: Vec<OsString> = path
            .components()
            .map(|c| c.as_os_str().to_os_string())
            .collect();
        if comps.is_empty() {
            return None;
        }
        let mut node = &mut self.root;
        for c in &comps[..comps.len() - 1] {
            node = node.children.get_mut(c)?;
        }
        node.children.remove(comps.last().unwrap());
        Some(())
    }

    /// Undo the counter increment from a previous [`FileNode::set_error`] call.
    fn decrement_error_counts(&mut self, path: &Path) {
        let mut node = &mut self.root;
        for c in path.components() {
            node.num_child_errors = node.num_child_errors.saturating_sub(1);
            let c = c.as_os_str().to_os_string();
            match node.children.get_mut(&c) {
                Some(child) => node = child,
                None => return,
            }
        }
    }
}

/// The longest common prefix of two relative paths.
pub fn lcp(a: &Path, b: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for (ca, cb) in a.components().zip(b.components()) {
        if ca.as_os_str() == cb.as_os_str() {
            out.push(ca.as_os_str());
        } else {
            break;
        }
    }
    out
}

/// The longest common ancestor of a collection of paths, i.e. the deepest directory that is an
/// ancestor of every path in `paths`.
///
/// Panics if `paths` is empty.
fn lca(paths: &[PathBuf]) -> PathBuf {
    match paths {
        [p] => p.parent().map(PathBuf::from).unwrap_or_default(),
        _ => paths.iter().fold(paths[0].clone(), |acc, p| lcp(&acc, p)),
    }
}
