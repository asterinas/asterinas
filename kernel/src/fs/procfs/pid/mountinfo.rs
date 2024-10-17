// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/mountinfo`.
/// See https://www.kernel.org/doc/Documentation/filesystems/proc.txt for details.
/// FIXME: Some fields are not implemented yet.
///
/// Fields:
/// - Mount ID: Unique identifier of the mount (mount id).
/// - Parent ID: The mount ID of the parent mount (or of self for the root of this mount namespace).
/// - Major:Minor: The device numbers of the device.
/// - Root: The pathname of the directory in the filesystem which forms the root of this mount.
/// - Mount Point: The pathname of the mount point relative to the process's root directory.
/// - Mount Options: Per-mount options.
/// - Optional Fields: Zero or more fields of the form "tag[:value]".
/// - FSType: The type of filesystem, such as ext3 or nfs.
/// - Source: The source of the mount.
/// - Super Options: Per-superblock options.
pub struct MountInfoFileOps(Arc<Process>);

impl MountInfoFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for MountInfoFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let process = &self.0;
        let root = process.fs().read().root().effective_name();
        let mount_point = if let Some(root_dentry) = process.fs().read().root().effective_parent() {
            root_dentry.effective_name()
        } else {
            "/".to_string()
        };

        let mountinfo_output = format!(
            "{}\t{}\t{}:{} {}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            0, // Mount ID
            0, // Parent ID
            0,
            0,           // Major:Minor
            root,        // Root
            mount_point, // Mount Point
            "unknown",   // Mount Options
            "none",      // Optional Fields
            "unknown",   // FSType
            "none",      // Source
            "unknown"    // Super Options
        );
        Ok(mountinfo_output.into_bytes())
    }
}
