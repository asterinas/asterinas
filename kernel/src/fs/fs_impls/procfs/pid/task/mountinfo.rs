// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        path::{Mount, Path, PathResolver, PerMountFlags},
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{FsFlags, Inode, mkmod},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// A helper function to create the mount point path for a given mount (used by `mounts` and `mountinfo`).
pub(super) fn make_mount_point_path(
    is_resolver_root_mount: bool,
    parent: Option<&Arc<Mount>>,
    mount: &Mount,
    path_resolver: &PathResolver,
) -> String {
    if is_resolver_root_mount {
        "/".to_string()
    } else if let Some(parent) = parent {
        if let Some(mount_point_dentry) = mount.mountpoint() {
            path_resolver
                .make_abs_path(&Path::new(parent.clone(), mount_point_dentry))
                .into_string()
        } else {
            "".to_string()
        }
    } else {
        // No parent means it's the root of the namespace.
        "/".to_string()
    }
}

/// A single entry in the mountinfo file.
struct MountInfoEntry<'a> {
    /// A unique ID for the mount (but not guaranteed to be unique across reboots).
    mount_id: usize,
    /// The ID of the parent mount (or self if it has no parent).
    parent_id: usize,
    /// The major device ID of the filesystem.
    major: u32,
    /// The minor device ID of the filesystem.
    minor: u32,
    /// The root of the mount within the filesystem.
    root: &'a str,
    /// The mount point relative to the process's root directory.
    mount_point: &'a str,
    /// Per-mount flags.
    mount_flags: PerMountFlags,
    /// The type of the filesystem in the form "type[.subtype]".
    fs_type: &'a str,
    /// Filesystem-specific information or "none".
    source: &'a str,
    /// Per-filesystem flags.
    fs_flags: FsFlags,
}

impl core::fmt::Display for MountInfoEntry<'_> {
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
            &self.mount_flags,
            &self.fs_type,
            &self.source,
            &self.fs_flags,
        )
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/mountinfo` (and also `/proc/[pid]/mountinfo`).
pub struct MountInfoFileOps(TidDirOps);

impl MountInfoFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3352>
        ProcFileBuilder::new(Self(dir.clone()), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }

    /// Reads mount information for `/proc/[pid]/mountinfo`.
    ///
    /// Provides detailed mount information including mount IDs, parent relationships,
    /// and device numbers.
    fn read_mount_info(
        &self,
        path_resolver: &PathResolver,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        for mount in path_resolver.collect_visible_mounts() {
            let mount_id = mount.id();
            let parent = mount.parent().and_then(|parent| parent.upgrade());
            let parent_id = parent.as_ref().map_or(mount_id, |p| p.id());
            let is_resolver_root_mount = Arc::ptr_eq(&mount, path_resolver.root().mount_node());
            let root = if is_resolver_root_mount {
                path_resolver.root().dentry().path_name()
            } else {
                mount.root_dentry().path_name()
            };
            let mount_point = make_mount_point_path(
                is_resolver_root_mount,
                parent.as_ref(),
                mount.as_ref(),
                path_resolver,
            );
            let mount_flags = mount.flags();
            let fs_type = mount.fs().name();
            let source = mount.source().unwrap_or("none");
            let fs_flags = mount.fs().flags();

            // The following fields are dummy for now.
            let major = 0;
            let minor = 0;

            let entry = MountInfoEntry {
                mount_id,
                parent_id,
                major,
                minor,
                root: &root,
                mount_point: &mount_point,
                mount_flags,
                fs_type,
                source,
                fs_flags,
            };

            writeln!(printer, "{}", entry)?;
        }

        Ok(printer.bytes_written())
    }
}

impl FileOps for MountInfoFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let fs = posix_thread.read_fs();
        let path_resolver = fs.resolver().read();
        self.read_mount_info(&path_resolver, offset, writer)
    }
}
