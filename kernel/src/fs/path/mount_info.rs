// SPDX-License-Identifier: MPL-2.0

use super::{mount::Mount, Path};
use crate::prelude::*;

/// A single entry in the mountinfo file.
struct MountInfoEntry {
    /// A unique ID for the mount (but not guaranteed to be unique across reboots).
    mount_id: usize,
    /// The ID of the parent mount (or self if it has no parent).
    parent_id: usize,
    /// The major device ID of the filesystem.
    major: u32,
    /// The minor device ID of the filesystem.
    minor: u32,
    /// The root of the mount within the filesystem.
    root: String,
    /// The mount point relative to the process's root directory.
    mount_point: String,
    /// Per-mount options.
    mount_options: String,
    /// The type of the filesystem in the form "type[.subtype]".
    fs_type: String,
    /// Filesystem-specific information or "none".
    source: String,
    /// Per-superblock options.
    super_options: String,
}

impl core::fmt::Display for MountInfoEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} {} {}:{} {} {} {} - {} {} {}",
            self.mount_id,
            self.parent_id,
            self.major,
            self.minor,
            &self.root,
            &self.mount_point,
            &self.mount_options,
            &self.fs_type,
            &self.source,
            &self.super_options
        )
    }
}

/// An abstraction for generating the content of /proc/self/mountinfo.
pub struct MountInfo {
    entries: Vec<MountInfoEntry>,
}

impl MountInfo {
    /// Creates a new `MountInfo` for the mount subtree of the given root.
    pub fn new(root_mount: &Arc<Mount>) -> Self {
        let mut entries = Vec::new();

        root_mount.traverse_with(|mount| {
            let mount_id = mount.id();
            let parent = mount.parent().and_then(|parent| parent.upgrade());
            let parent_id = parent.as_ref().map_or(mount_id, |p| p.id());

            let root = Path::new_fs_root(mount.clone()).abs_path();

            let mount_point = if let Some(parent) = parent {
                if let Some(mount_point_dentry) = mount.mountpoint() {
                    Path::new(parent, mount_point_dentry).abs_path()
                } else {
                    "".to_string()
                }
            } else {
                // No parent means it's the root of the namespace.
                "/".to_string()
            };

            let fs_type = mount.fs().name().to_string();

            // The following fields are dummy for now.
            let major = 0;
            let minor = 0;
            let mount_options = "rw,relatime".to_string();
            let source = "none".to_string();
            let super_options = "rw".to_string();

            entries.push(MountInfoEntry {
                mount_id,
                parent_id,
                major,
                minor,
                root,
                mount_point,
                mount_options,
                fs_type,
                source,
                super_options,
            });
        });

        Self { entries }
    }
}

impl core::fmt::Display for MountInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for entry in &self.entries {
            writeln!(f, "{}", entry)?;
        }
        Ok(())
    }
}
