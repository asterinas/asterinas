// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use aster_util::{printer::VmPrinter, slot_vec::SlotVec};
use ostd::sync::RwMutexUpgradeableGuard;

use super::TidDirOps;
use crate::{
    fs::{
        file_handle::FileLike,
        file_table::FileDesc,
        procfs::{
            DirOps, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps,
            template::{FileOps, ProcFile, ProcFileBuilder, ProcSym},
        },
        utils::{AccessMode, DirEntryVecExt, Inode, SymbolicLink, chmod, mkmod},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/fd` (and also `/proc/[pid]/fd`).
pub(super) struct FdDirOps<T> {
    dir: TidDirOps,
    marker: PhantomData<T>,
}

impl<T: FdOps> FdDirOps<T> {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(
            Self {
                dir: dir.clone(),
                marker: PhantomData,
            },
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3317>
            mkmod!(u+rx),
        )
        .parent(parent)
        .build()
        .unwrap()
    }
}

impl<T: FdOps> DirOps for FdDirOps<T> {
    // Lock order: cached entries -> file table
    //
    // Note that inverting the lock order is non-trivial because the file table is protected by a
    // spin lock but the cached entries are protected by a mutex.

    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(file_desc) = name.parse::<FileDesc>() else {
            return_errno_with_message!(Errno::ENOENT, "the name is not a valid FD");
        };

        let mut cached_children = dir.cached_children().write();

        let thread = self.dir.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let access_mode = if let Some(file_table) = posix_thread.file_table().lock().as_ref()
            && let Ok(file) = file_table.read().get_file(file_desc)
        {
            file.access_mode()
        } else {
            return_errno_with_message!(Errno::ENOENT, "the file does not exist");
        };

        let child = T::new_inode(
            self.dir.clone(),
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

        let thread = self.dir.thread();
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
            let child = child.downcast_ref::<T::NodeType>().unwrap();
            let child_ops = T::ref_from_inode(child);

            let Ok(file) = file_table.get_file(child_ops.file_desc()) else {
                cached_children.remove(i);
                continue;
            };
            if !child_ops.is_valid(file) {
                cached_children.remove(i);
            }
        }

        // Add new entries.
        for (file_desc, file) in file_table.fds_and_files() {
            cached_children.put_entry_if_not_found(&file_desc.to_string(), || {
                T::new_inode(
                    self.dir.clone(),
                    file_desc,
                    file.access_mode(),
                    dir.this_weak().clone(),
                )
            });
        }

        cached_children.downgrade()
    }

    fn validate_child(&self, child: &dyn Inode) -> bool {
        let child = child.downcast_ref::<T::NodeType>().unwrap();
        let child_ops = T::ref_from_inode(child);

        let thread = self.dir.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        if let Some(file_table) = posix_thread.file_table().lock().as_ref()
            && let Ok(file) = file_table.read().get_file(child_ops.file_desc())
        {
            child_ops.is_valid(file)
        } else {
            false
        }
    }
}

pub(super) trait FdOps: Send + Sync + 'static {
    type NodeType: Inode;

    fn new_inode(
        tid_dir_ops: TidDirOps,
        file_desc: FileDesc,
        access_mode: AccessMode,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode>;

    fn file_desc(&self) -> FileDesc;

    fn is_valid(&self, correspond_file: &Arc<dyn FileLike>) -> bool;

    fn ref_from_inode(inode: &Self::NodeType) -> &Self;
}

/// Represents the inode at `/proc/[pid]/task/[tid]/fd/[n]` (and also `/proc/[pid]/fd/[n]`).
pub(super) struct FileSymOps {
    tid_dir_ops: TidDirOps,
    file_desc: FileDesc,
    access_mode: AccessMode,
}

impl FdOps for FileSymOps {
    type NodeType = ProcSym<FileSymOps>;

    fn new_inode(
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

    fn file_desc(&self) -> FileDesc {
        self.file_desc
    }

    fn is_valid(&self, correspond_file: &Arc<dyn FileLike>) -> bool {
        // We'll treat the old entry as valid and reuse it if the access mode is the same,
        // even if the file is different.
        self.access_mode == correspond_file.access_mode()
    }

    fn ref_from_inode(inode: &Self::NodeType) -> &Self {
        inode.inner()
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

        Ok(SymbolicLink::Path(file.path().clone()))
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]/fdinfo/[n]` (and also `/proc/[pid]/fdinfo/[n]`).
pub(super) struct FileInfoOps {
    tid_dir_ops: TidDirOps,
    file_desc: FileDesc,
}

impl FdOps for FileInfoOps {
    type NodeType = ProcFile<FileInfoOps>;

    fn new_inode(
        tid_dir_ops: TidDirOps,
        file_desc: FileDesc,
        _access_mode: AccessMode,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode> {
        ProcFileBuilder::new(
            Self {
                tid_dir_ops,
                file_desc,
            },
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/fd.c#L383>.
            mkmod!(a+r),
        )
        .parent(parent)
        .build()
        .unwrap()
    }

    fn file_desc(&self) -> FileDesc {
        self.file_desc
    }

    fn is_valid(&self, _correspond_file: &Arc<dyn FileLike>) -> bool {
        true
    }

    fn ref_from_inode(inode: &Self::NodeType) -> &Self {
        inode.inner()
    }
}

impl FileOps for FileInfoOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let thread = self.tid_dir_ops.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        let info = if let Some(file_table) = posix_thread.file_table().lock().as_ref()
            && let Ok(entry) = file_table.read().get_entry(self.file_desc)
        {
            entry.file().clone().dump_proc_fdinfo(entry.flags())
        } else {
            return_errno_with_message!(Errno::ENOENT, "the file does not exist");
        };

        let mut printer = VmPrinter::new_skip(writer, offset);
        write!(printer, "{}", info)?;

        Ok(printer.bytes_written())
    }
}
