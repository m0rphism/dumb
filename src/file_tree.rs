use std::{
    collections::HashMap,
    ffi::OsString,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::scan::{Inode, InodeId, PathType, ScanEvent};

#[derive(Debug, Clone)]
pub struct FileTree {
    pub inodes: HashMap<InodeId, Inode>,
    pub root: FileNode,
}

#[derive(Debug, Clone)]
pub struct FileNode {
    pub children: HashMap<OsString, FileNode>,
    pub inode_id: InodeId,
    pub state: FileState,
}

#[derive(Debug, Clone)]
pub enum FileState {
    Queued,
    InProgress,
    Finished,
    Failed(String),
}

impl FileNode {
    pub fn new(inode_id: u64) -> Self {
        Self {
            children: HashMap::new(),
            state: FileState::Queued,
            inode_id,
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

    pub fn set_queued(&mut self, path: &Path, inode_id: u64) {
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

    pub fn set_error(&mut self, path: &Path, error: String) {
        self.get_mut(path).unwrap().state = FileState::Failed(error);
    }

    pub fn finish_file(
        &mut self,
        path: PathBuf,
        inode: Inode,
        inodes: &mut HashMap<InodeId, Inode>,
    ) {
        let mut node = self;
        for c in path.components() {
            if !inode.is_dir {
                match node.state {
                    FileState::Queued | FileState::InProgress => {
                        node.state = FileState::InProgress;
                        let inode2 = inodes.get_mut(&node.inode_id).unwrap();
                        inode2.size_in_bytes += inode.size_in_bytes;
                        inode2.last_modified =
                            SystemTime::max(inode2.last_modified, inode.last_modified);
                    }
                    _ => unreachable!(),
                }
            }
            node = node
                .children
                .get_mut(&c.as_os_str().to_os_string())
                .unwrap()
        }
        node.state = FileState::Finished;
    }
}

impl FileTree {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            inodes: HashMap::new(),
            root: FileNode::new(path.as_ref().metadata().unwrap().ino()),
        }
    }

    pub fn get(&self, path: &Path) -> Option<&FileNode> {
        self.root.get(path)
    }

    pub fn get_mut(&mut self, path: &Path) -> Option<&mut FileNode> {
        self.root.get_mut(path)
    }

    pub fn handle_scan_event(&mut self, scan_event: ScanEvent) {
        match scan_event {
            ScanEvent::StartedScan(path, inode_id, path_type) => {
                self.root.set_queued(&path, inode_id);
                self.inodes.insert(
                    inode_id,
                    Inode {
                        size_in_bytes: 0,
                        last_modified: SystemTime::UNIX_EPOCH,
                        num_hardlinks: 0,
                        is_dir: path_type == PathType::Dir,
                    },
                );
            }
            ScanEvent::NewInode(inode_id, inode) => {
                self.inodes.insert(inode_id, inode);
            }
            ScanEvent::FinishedScan(path, inode_id) => self.root.finish_file(
                path,
                *self.inodes.get(&inode_id).unwrap(),
                &mut self.inodes,
            ),
            ScanEvent::FailedScan(path, err) => self.root.set_error(&path, err),
        }
    }
}
