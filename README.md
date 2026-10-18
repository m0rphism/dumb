# dumb

An interactive TUI version of the standard unix `du` program written in Rust.

Recursively scans a directory and presents an interactive file tree that
highlights the disk usage caused by each entry.

Useful for exploring what eats up all that disk space.

## Core Features

- **Concurrency.**
  Live file tree updates while the directory is being scanned recursively.
  This allows exploring subdirectories before the scan is completed.

- **Proper Size Semantics.**
  The size of a file system entry is displayed as the disk space that would
  become available by deleting the entry.
  In particular, this accounts for hardlinks and uses actual block sizes
  instead of the apparent sizes of files.
  In corner cases or exotic environments (e.g. network mounts) the size may be
  slightly off, see the Limitations section for details.

- **Keybindings.**
  Both `vim` keybindings and regular keybindings are supported.
  Press `?` inside the program to display a list of all keybindings.

- **In-Process Deletion.**
  Pressing `d` deletes the currently selected entry (by default with confirmation).
  Deletion is also possible while the deleted directory is still being scanned,
  and the sizes of other directories are adjusted correctly even in the
  presence of hardlinks.

- **TOML config and themes.**
  Many behaviors can be customized and virtually everything can be themed.
  If you are missing a config/theme option feel free to open an issue.

- **Error Display.**
  File entries which could not be scanned, e.g. missing permissions to access a
  subdirectory, are highlighted in the file tree. The total error count is
  displayed in the header line.
  Pressing `E` displays all errors. Pressing `e` on a file entry displays only
  the errors that occured while scanning this particular entry.

## Contributing

I am generally open for contributions, but ask that you adhere to a single rule:

> **Issues and pull request must start with at least one human written paragraph,
> and AI generated text and code must be clearly highlighted as such.**

Pure slop issues/pull requests might be closed without any response.

## Limitations

- Unix-only as `std::os::unix::fs::MetadataExt` is used.
- Files deleted outside of the `dumb` process are not detected.
- The `freed` size of an entry is computed from its allocated blocks
  (`st_blocks`, like `du`) and can slightly differ from the space that deleting
  it actually frees:
  - Filesystem metadata (inodes, extent trees, journal) is not counted,
    so sizes are slightly underestimated.
  - On filesystems with reflink, snapshots, or deduplication (e.g. btrfs,
    ZFS), data that is shared between entries is reported as fully
    allocated, but deleting one entry may free none of it.
  - Recently written files may report fewer allocated blocks until the
    filesystem has written their data back.
  - Pseudo filesystems and some network or FUSE mounts (e.g. CIFS/SMB)
    may report zero or approximate block counts.
