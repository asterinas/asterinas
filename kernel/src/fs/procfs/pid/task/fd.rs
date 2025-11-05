// SPDX-License-Identifier: MPL-2.0

use aster_util::slot_vec::SlotVec;
use ostd::sync::RwMutexUpgradeableGuard;

use super::TidDirOps;
use crate::{
    fs::{
        file_table::FileDesc,
        inode_handle::InodeHandle,
        procfs::{template::ProcSym, DirOps, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps},
        utils::{chmod, mkmod, AccessMode, DirEntryVecExt, Inode, SymbolicLink},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/fd` (and also `/proc/[pid]/fd`).
pub struct FdDirOps(TidDirOps);

impl FdDirOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3317>
        ProcDirBuilder::new(Self(dir.clone()), mkmod!(u+rx))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl DirOps for FdDirOps {
    // Lock order: cached entries -> file table
    //
    // Note that inverting the lock order is non-trivial because the file table is protected by a
    // spin lock but the cached entries are protected by a mutex.

    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(file_desc) = name.parse::<FileDesc>() else {
            return_errno_with_message!(Errno::ENOENT, "the name is not a valid FD");
        };

        let mut cached_children = dir.cached_children().write();

        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let access_mode = if let Some(file_table) = posix_thread.file_table().lock().as_ref()
            && let Ok(file) = file_table.read().get_file(file_desc)
        {
            file.access_mode()
        } else {
            return_errno_with_message!(Errno::ENOENT, "the file does not exist");
        };

        let child = FileSymOps::new_inode(
            self.0.clone(),
            file_desc,
            access_mode,
            dir.this_weak().clone(),
        );
        // The old entry is likely outdated given that `lookup_child` is called. Race conditions
        // may occur, but caching the file descriptor (which aligns with the Linux implementation)
        // is inherently racy, so preventing race conditions is not very meaningful.
        cached_children.remove_entry_by_name(name);
        cached_children.put((String::from(name), child.clone()));

        Ok(child)
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let mut cached_children = dir.cached_children().write();

        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let file_table = posix_thread.file_table().lock();
        let Some(file_table) = file_table.as_ref() else {
            *cached_children = SlotVec::new();
            return cached_children.downgrade();
        };

        let file_table = file_table.read();

        // Remove outdated entries.
        for i in 0..cached_children.slots_len() {
            let Some((_, child)) = cached_children.get(i) else {
                continue;
            };
            let child = child.downcast_ref::<ProcSym<FileSymOps>>().unwrap();

            let Ok(file) = file_table.get_file(child.inner().file_desc) else {
                cached_children.remove(i);
                continue;
            };
            if file.access_mode() != child.inner().access_mode {
                cached_children.remove(i);
            }
            // We'll reuse the old entry if the access mode is the same, even if the file is
            // different.
        }

        // Add new entries.
        for (file_desc, file) in file_table.fds_and_files() {
            cached_children.put_entry_if_not_found(&file_desc.to_string(), || {
                FileSymOps::new_inode(
                    self.0.clone(),
                    file_desc,
                    file.access_mode(),
                    dir.this_weak().clone(),
                )
            });
        }

        cached_children.downgrade()
    }

    fn validate_child(&self, child: &dyn Inode) -> bool {
        let ops = child.downcast_ref::<ProcSym<FileSymOps>>().unwrap();

        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let is_valid = if let Some(file_table) = posix_thread.file_table().lock().as_ref()
            && let Ok(file) = file_table.read().get_file(ops.inner().file_desc)
        {
            // We'll reuse the old entry if the access mode is the same, even if the file is
            // different.
            file.access_mode() == ops.inner().access_mode
        } else {
            false
        };

        is_valid
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/fd/[n]` (and also `/proc/[pid]/fd/[n]`).
struct FileSymOps {
    tid_dir_ops: TidDirOps,
    file_desc: FileDesc,
    access_mode: AccessMode,
}

impl FileSymOps {
    pub fn new_inode(
        tid_dir_ops: TidDirOps,
        file_desc: FileDesc,
        access_mode: AccessMode,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/fd.c#L127-L141>
        let mut mode = mkmod!(a=);
        if access_mode.is_readable() {
            mode = chmod!(mode, u+rx);
        }
        if access_mode.is_writable() {
            mode = chmod!(mode, u+wx);
        }

        ProcSymBuilder::new(
            Self {
                tid_dir_ops,
                file_desc,
                access_mode,
            },
            mode,
        )
        .parent(parent)
        .build()
        .unwrap()
    }
}

impl SymOps for FileSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        let thread = self.tid_dir_ops.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let file_table = posix_thread.file_table().lock();
        let Some(file_table) = file_table.as_ref() else {
            return_errno_with_message!(Errno::ENOENT, "the thread has exited");
        };
        let file_table = file_table.read();
        let file = file_table
            .get_file(self.file_desc)
            .map_err(|_| Error::with_message(Errno::ENOENT, "the file does not exist"))?;

        let res = if let Some(inode_handle) = file.downcast_ref::<InodeHandle>() {
            SymbolicLink::Path(inode_handle.path().clone())
        } else {
            SymbolicLink::Inode(file.inode().clone())
        };

        Ok(res)
    }
}
