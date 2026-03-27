// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::{AccessMode, FileLike, chmod, file_table::FileDesc, mkmod},
        procfs::template::{
            DirOps, FileOps, ProcDir, ProcDirBuilder, ProcFile, ProcFileBuilder, ProcSym,
            ProcSymBuilder, ReaddirEntry, SymOps, keyed_readdir_entries,
        },
        vfs::inode::{Inode, SymbolicLink},
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
        .need_neg_child_revalidation()
        .build()
        .unwrap()
    }

    fn fd_entries(&self, dir: &ProcDir<Self>) -> Vec<(usize, String, Arc<dyn Inode>)> {
        let thread = self.dir.thread();
        let posix_thread = thread.as_posix_thread().unwrap();
        let file_table = posix_thread.file_table().lock();
        let Some(file_table) = file_table.as_ref() else {
            return Vec::new();
        };

        file_table
            .read()
            .fds_and_files()
            .filter_map(|(file_desc, file)| {
                usize::try_from(file_desc).ok().map(|fd| {
                    let inode = T::new_inode(
                        self.dir.clone(),
                        file_desc,
                        file.access_mode(),
                        dir.this_weak().clone(),
                    );
                    (fd, file_desc.to_string(), inode)
                })
            })
            .collect()
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

        let thread = self.dir.thread();
        let posix_thread = thread.as_posix_thread().unwrap();
        let file_table = posix_thread.file_table().lock();

        let access_mode = if let Some(file_table) = file_table.as_ref()
            && let Ok(file) = file_table.read().get_file(file_desc)
        {
            file.access_mode()
        } else {
            return_errno_with_message!(Errno::ENOENT, "the file does not exist");
        };

        Ok(T::new_inode(
            self.dir.clone(),
            file_desc,
            access_mode,
            dir.this_weak().clone(),
        ))
    }

    fn populate_children(&self, dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)> {
        self.fd_entries(dir)
            .into_iter()
            .map(|(_, name, inode)| (name, inode))
            .collect()
    }

    fn entries_from_offset(&self, dir: &ProcDir<Self>, offset: usize) -> Vec<ReaddirEntry> {
        keyed_readdir_entries(offset, 2, self.fd_entries(dir))
    }

    fn revalidate_pos_child(&self, _name: &str, child: &dyn Inode) -> bool {
        let child = child.downcast_ref::<T::NodeType>().unwrap();
        let child_ops = T::ref_from_inode(child);

        let thread = self.dir.thread();
        let posix_thread = thread.as_posix_thread().unwrap();
        let file_table = posix_thread.file_table().lock();

        if let Some(file_table) = file_table.as_ref()
            && let Ok(file) = file_table.read().get_file(child_ops.file_desc())
        {
            child_ops.is_valid(file)
        } else {
            false
        }
    }

    fn revalidate_neg_child(&self, name: &str) -> bool {
        let Ok(file_desc) = name.parse::<FileDesc>() else {
            return true;
        };

        let thread = self.dir.thread();
        let posix_thread = thread.as_posix_thread().unwrap();
        let file_table = posix_thread.file_table().lock();

        let Some(file_table) = file_table.as_ref() else {
            return true;
        };

        file_table.read().get_file(file_desc).is_err()
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
        .need_revalidation()
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
        .need_revalidation()
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
        let file_table = posix_thread.file_table().lock();

        let info = if let Some(file_table) = file_table.as_ref()
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
