// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        file_handle::FileLike,
        file_table::FileDesc,
        inode_handle::InodeHandle,
        procfs::{
            pid::FdEvents, DirOps, Observer, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps,
        },
        utils::{DirEntryVecExt, Inode, InodeMode},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/fd` (and also `/proc/[pid]/fd`).
pub struct FdDirOps(Arc<Thread>);

impl FdDirOps {
    pub fn new_inode(thread_ref: Arc<Thread>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let posix_thread = thread_ref.as_posix_thread().unwrap();
        let file_table = posix_thread.file_table();

        let fd_inode = ProcDirBuilder::new(
            Self(thread_ref.clone()),
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3317>
            InodeMode::from_bits_truncate(0o500),
        )
        .parent(parent)
        .build()
        .unwrap();
        // This is for an exiting process that has not yet been reaped by its parent,
        // whose file table may have already been released.
        if let Some(file_table_ref) = file_table.lock().as_ref() {
            file_table_ref
                .read()
                .register_observer(Arc::downgrade(&fd_inode) as _);
        }

        fd_inode
    }
}

impl Observer<FdEvents> for ProcDir<FdDirOps> {
    fn on_events(&self, events: &FdEvents) {
        let fd_string = if let FdEvents::Close(fd) = events {
            fd.to_string()
        } else {
            return;
        };

        let mut cached_children = self.cached_children().write();
        cached_children.remove_entry_by_name(&fd_string);
    }
}

impl DirOps for FdDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let posix_thread = self.0.as_posix_thread().unwrap();
        let file_table = posix_thread.file_table().lock();
        let file_table = file_table
            .as_ref()
            .ok_or_else(|| Error::new(Errno::ENOENT))?;

        let file = {
            let fd = name
                .parse::<FileDesc>()
                .map_err(|_| Error::new(Errno::ENOENT))?;
            file_table
                .read()
                .get_file(fd)
                .map_err(|_| Error::new(Errno::ENOENT))?
                .clone()
        };

        Ok(FileSymOps::new_inode(file, this_ptr.clone()))
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let posix_thread = self.0.as_posix_thread().unwrap();
        let file_table = posix_thread.file_table().lock();
        let Some(file_table) = file_table.as_ref() else {
            return;
        };

        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<FdDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();

        for (fd, file) in file_table.read().fds_and_files() {
            cached_children.put_entry_if_not_found(&fd.to_string(), || {
                FileSymOps::new_inode(file.clone(), this_ptr.clone())
            });
        }
    }
}

/// Represents the inode at `/proc/[pid]/fd/N`.
struct FileSymOps(Arc<dyn FileLike>);

impl FileSymOps {
    pub fn new_inode(file: Arc<dyn FileLike>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/fd.c#L127-L141>
        let mut mode = InodeMode::empty();
        if file.access_mode().is_readable() {
            mode |= InodeMode::S_IRUSR | InodeMode::S_IXUSR;
        }
        if file.access_mode().is_writable() {
            mode |= InodeMode::S_IWUSR | InodeMode::S_IXUSR;
        }

        ProcSymBuilder::new(Self(file), mode)
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl SymOps for FileSymOps {
    fn read_link(&self) -> Result<String> {
        let path_name = if let Some(inode_handle) = self.0.downcast_ref::<InodeHandle>() {
            inode_handle.path().abs_path()
        } else {
            // TODO: get the real path for other FileLike object
            String::from("/dev/tty")
        };
        Ok(path_name)
    }
}
