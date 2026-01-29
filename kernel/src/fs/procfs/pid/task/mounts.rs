// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        path::{Path, PathResolver, PerMountFlags},
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// A single entry in the mounts file.
struct MountsEntry<'a> {
    /// Filesystem-specific information or "none".
    source: &'a str,
    /// Mount point relative to the process's root directory.
    mount_point: &'a str,
    /// The type of the filesystem in the form "type[.subtype]".
    fs_type: &'a str,
    /// Per-mount flags.
    mount_flags: PerMountFlags,
    /// The dump field is used by the dump(8) program to determine which
    /// filesystems need to be dumped.
    dump: u32,
    /// The fsck(8) program uses this field.
    pass: u32,
}

impl core::fmt::Display for MountsEntry<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {}",
            &self.source,
            &self.mount_point,
            &self.fs_type,
            &self.mount_flags,
            &self.dump,
            &self.pass,
        )
    }
}
pub struct MountsFileOps(TidDirOps);

impl MountsFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3351>
        ProcFileBuilder::new(Self(dir.clone()), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }

    /// Reads mount information for `/proc/[pid]/mounts` and `/proc/mounts`.
    ///
    /// Provides a simplified view of mounted filesystems in the traditional
    /// `/etc/fstab` format.
    pub fn read_mounts(
        &self,
        path_resolver: &PathResolver,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        for mount in path_resolver.collect_visible_mounts() {
            let parent = mount.parent().and_then(|parent| parent.upgrade());
            let is_resolver_root_mount = Arc::ptr_eq(&mount, path_resolver.root().mount_node());
            let mount_point = if is_resolver_root_mount {
                "/".to_string()
            } else if let Some(parent) = parent {
                if let Some(mount_point_dentry) = mount.mountpoint() {
                    path_resolver
                        .make_abs_path(&Path::new(parent, mount_point_dentry))
                        .into_string()
                } else {
                    "".to_string()
                }
            } else {
                // No parent means it's the root of the namespace.
                "/".to_string()
            };
            let mount_flags = mount.flags();
            let fs_type = mount.fs().name();
            let source = mount.source().unwrap_or("none");

            let entry = MountsEntry {
                source,
                mount_point: &mount_point,
                fs_type,
                mount_flags,
                dump: 0,
                pass: 0,
            };

            writeln!(printer, "{}", entry)?;
        }

        Ok(printer.bytes_written())
    }
}

impl FileOps for MountsFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let fs = posix_thread.read_fs();
        let path_resolver = fs.resolver().read();
        self.read_mounts(&path_resolver, offset, writer)
    }
}
