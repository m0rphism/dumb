//! Mount point detection in paths (linux-only)

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// The answer to a search for mount points. Returned by [`find_mount_point`].
#[derive(Debug, PartialEq, Eq)]
pub enum Search {
    /// The path is no mount point and contains no mount point.
    NoMountPoint,
    /// The path is a mount point or contains a mount point.
    MountPoint(PathBuf),
    /// The system provides no mount table, so mount points could not be determined.
    NoMountTable,
}

/// Check if path `p` is a mount point or contains a mountpoint.
///
/// Uses the `/proc/self/mounts` mount table and returns `Ok(Search::NoMountTable)` for systems
/// which do not support it.
pub fn find_mount_point(p: &Path) -> Result<Search, std::io::Error> {
    // A symlink is never a mount point.
    if std::fs::symlink_metadata(p)?.file_type().is_symlink() {
        return Ok(Search::NoMountPoint);
    }

    // Resolve symlinks so the path can be compared against the mount point paths, which
    // are already resolved.
    let p = std::fs::canonicalize(p)?;

    // Read the mount table.
    let content = match std::fs::read_to_string("/proc/self/mounts") {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Search::NoMountTable),
        Err(e) => return Err(e),
    };

    // Each line of `/proc/self/mounts` has the form
    // `<device> <mount point> <fs type> <options> <dump freq> <fs check>`.
    let mounts = content
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(decode_mount_point)
        .collect::<Vec<_>>();

    Ok(match find_mount_point_in(&p, &mounts) {
        Some(m) => Search::MountPoint(m.clone()),
        None => Search::NoMountPoint,
    })
}

/// Returns the mount point that is equal to `p` or located under `p`, if any. An exact
/// match takes precedence over a contained mount point, so that a path which is itself
/// a mount point is reported as such.
fn find_mount_point_in<'a>(p: &Path, mounts: &'a [PathBuf]) -> Option<&'a PathBuf> {
    mounts
        .iter()
        .find(|m| m.as_path() == p)
        .or_else(|| mounts.iter().find(|m| m.starts_with(p)))
}

/// Decodes the octal escapes (`\040` = space, `\011` = tab, `\012` = newline, `\134` =
/// backslash) that the kernel uses for special characters in mount point paths in
/// `/proc/self/mounts`.
fn decode_mount_point(s: &str) -> PathBuf {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let octal: String = chars.by_ref().take(3).collect();
            if octal.len() == 3 {
                let code = u32::from_str_radix(&octal, 8).unwrap_or(0xFFFD);
                out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
            } else {
                out.push('\\');
                out.push_str(&octal);
            }
        } else {
            out.push(c);
        }
    }
    PathBuf::from(out)
}
