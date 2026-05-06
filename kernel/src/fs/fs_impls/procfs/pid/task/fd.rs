// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::{
            AccessMode, FileLike, InodeType, chmod,
            file_table::{FileDesc, RawFileDesc},
            mkmod,
        },
        procfs::template::{
            DirOps, FileOps, ListedEntry, ProcDir, ProcFile, ProcSym, ReaddirEntry, SymOps,
            keyed_readdir_entries, visit_readdir_entries,
        },
        vfs::inode::{Inode, RevalidationPolicy, SymbolicLink},
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
        ProcDir::new(
            Self {
                dir: dir.clone(),
                marker: PhantomData,
            },
            parent,
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3317>
            mkmod!(u+rx),
        )
    }
}

impl<T: FdOps> DirOps for FdDirOps<T> {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let file_desc = if let Ok(raw_fd) = name.parse::<RawFileDesc>()
            && let Ok(file_desc) = FileDesc::try_from(raw_fd)
        {
            file_desc
        } else {
            return_errno_with_message!(Errno::ENOENT, "the name is not a valid FD");
        };

        let Some(thread) = self.dir.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
        let posix_thread = thread.as_posix_thread().unwrap();

        let access_mode = if let Some(file_table) = posix_thread.file_table().lock().as_ref()
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
            this_dir.this_weak().clone(),
        ))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        let Some(thread) = self.dir.thread() else {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        };
        let posix_thread = thread.as_posix_thread().unwrap();

        let file_table = posix_thread.file_table().lock();
        let Some(file_table) = file_table.as_ref() else {
            // The thread has exited.
            return Ok(());
        };

        // Collect the list of FDs first before visiting entries. This can avoid holding the file
        // table lock while calling `visit_fn`, which may break the atomic mode.
        let file_descs = {
            let file_table = file_table.read();
            file_table
                .fds_and_files()
                .map(|(file_desc, _)| usize::from(file_desc))
                .collect::<Vec<_>>()
        };

        visit_readdir_entries(
            keyed_readdir_entries(offset, 2, file_descs, |file_desc| {
                ListedEntry::new(file_desc.to_string(), T::entry_inode_type())
            }),
            visit_fn,
        )
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
    }

    fn revalidate_exists(&self, _name: &str, child: &dyn Inode) -> bool {
        let child = child.downcast_ref::<T::NodeType>().unwrap();
        let child_ops = T::ref_from_inode(child);

        let Some(thread) = self.dir.thread() else {
            return false;
        };
        let posix_thread = thread.as_posix_thread().unwrap();

        if let Some(file_table) = posix_thread.file_table().lock().as_ref()
            && let Ok(file) = file_table.read().get_file(child_ops.file_desc())
        {
            child_ops.is_valid(file)
        } else {
            false
        }
    }

    fn revalidate_absent(&self, name: &str) -> bool {
        let Ok(file_desc) = name.parse::<RawFileDesc>() else {
            return true;
        };
        let Ok(file_desc) = file_desc.try_into() else {
            return true;
        };

        let Some(thread) = self.dir.thread() else {
            return true;
        };
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

    fn entry_inode_type() -> InodeType;

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

        ProcSym::new(
            Self {
                tid_dir_ops,
                file_desc,
                access_mode,
            },
            parent,
            mode,
        )
    }

    fn file_desc(&self) -> FileDesc {
        self.file_desc
    }

    fn entry_inode_type() -> InodeType {
        InodeType::SymLink
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
        let Some(thread) = self.tid_dir_ops.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
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
        ProcFile::new(
            Self {
                tid_dir_ops,
                file_desc,
            },
            parent,
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/fd.c#L383>.
            mkmod!(a+r),
        )
    }

    fn file_desc(&self) -> FileDesc {
        self.file_desc
    }

    fn entry_inode_type() -> InodeType {
        InodeType::File
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
        let Some(thread) = self.tid_dir_ops.thread() else {
            return_errno_with_message!(Errno::ESRCH, "the thread does not exist");
        };
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
