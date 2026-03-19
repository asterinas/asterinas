// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::{
            pid::task::mountinfo::make_mount_point_path,
            template::{FileOps, ProcFileBuilder},
        },
        vfs::{inode::Inode, path::PathResolver},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// A single entry in the `mountstats` file.
struct MountStatsEntry<'a> {
    /// Filesystem-specific information or `"none"`.
    source: &'a str,
    /// Mount point relative to the process's root directory.
    mount_point: &'a str,
    /// The type of the filesystem.
    fs_type: &'a str,
}

impl core::fmt::Display for MountStatsEntry<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "device {} mounted on {} with fstype {}",
            self.source, self.mount_point, self.fs_type
        )
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/mountstats` (and also `/proc/[pid]/mountstats`).
pub struct MountStatsFileOps(TidDirOps);

impl MountStatsFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(dir.clone()), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }

    /// Reads mount statistics for `/proc/[pid]/mountstats`.
    ///
    /// Linux always exposes a per-mount summary line and only appends extra
    /// statistics for filesystems that maintain them, such as NFS.
    fn read_mount_stats(
        &self,
        path_resolver: &PathResolver,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        for mount in path_resolver.collect_visible_mounts() {
            let parent = mount.parent().and_then(|parent| parent.upgrade());
            let is_resolver_root_mount = Arc::ptr_eq(&mount, path_resolver.root().mount_node());
            let mount_point = make_mount_point_path(
                is_resolver_root_mount,
                parent.as_ref(),
                mount.as_ref(),
                path_resolver,
            );
            let source = mount.source().unwrap_or("none");
            let fs_type = mount.fs().name();
            let entry = MountStatsEntry {
                source,
                mount_point: &mount_point,
                fs_type,
            };

            writeln!(printer, "{}", entry)?;
        }

        Ok(printer.bytes_written())
    }
}

impl FileOps for MountStatsFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let fs = posix_thread.read_fs();
        let path_resolver = fs.resolver().read();
        self.read_mount_stats(&path_resolver, offset, writer)
    }
}
