use crate::events::{Events, Observer, Subject};
use crate::prelude::*;

use super::{
    file_handle::FileHandle,
    stdio::{Stderr, Stdin, Stdout, FD_STDERR, FD_STDIN, FD_STDOUT},
};

pub type FileDescripter = i32;

pub struct FileTable {
    table: BTreeMap<FileDescripter, FileHandle>,
    subject: Subject<FdEvents>,
}

impl FileTable {
    pub fn new() -> Self {
        Self {
            table: BTreeMap::new(),
            subject: Subject::new(),
        }
    }

    pub fn new_with_stdio() -> Self {
        let mut table = BTreeMap::new();
        let stdin = Stdin::new_with_default_console();
        let stdout = Stdout::new_with_default_console();
        let stderr = Stderr::new_with_default_console();
        table.insert(FD_STDIN, FileHandle::new_file(Arc::new(stdin)));
        table.insert(FD_STDOUT, FileHandle::new_file(Arc::new(stdout)));
        table.insert(FD_STDERR, FileHandle::new_file(Arc::new(stderr)));
        Self {
            table,
            subject: Subject::new(),
        }
    }

    pub fn dup(&mut self, fd: FileDescripter, new_fd: Option<FileDescripter>) -> Result<()> {
        let file = self.table.get(&fd).map_or_else(
            || return_errno_with_message!(Errno::ENOENT, "No such file"),
            |f| Ok(f.clone()),
        )?;
        let new_fd = if let Some(new_fd) = new_fd {
            new_fd
        } else {
            self.max_fd() + 1
        };
        if self.table.contains_key(&new_fd) {
            return_errno_with_message!(Errno::EBADF, "Fd exists");
        }
        self.table.insert(new_fd, file);

        Ok(())
    }

    fn max_fd(&self) -> FileDescripter {
        self.table.iter().map(|(fd, _)| fd.clone()).max().unwrap()
    }

    pub fn insert(&mut self, item: FileHandle) -> FileDescripter {
        let fd = self.max_fd() + 1;
        self.table.insert(fd, item);
        fd
    }

    pub fn insert_at(&mut self, fd: FileDescripter, item: FileHandle) -> Option<FileHandle> {
        let file = self.table.insert(fd, item);
        if file.is_some() {
            self.notify_close_fd_event(fd);
        }
        file
    }

    pub fn close_file(&mut self, fd: FileDescripter) -> Option<FileHandle> {
        let file = self.table.remove(&fd);
        if file.is_some() {
            self.notify_close_fd_event(fd);
        }
        file
    }

    pub fn get_file(&self, fd: FileDescripter) -> Result<&FileHandle> {
        self.table
            .get(&fd)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }

    pub fn fds_and_files(&self) -> impl Iterator<Item = (&'_ FileDescripter, &'_ FileHandle)> {
        self.table.iter()
    }

    pub fn register_observer(&self, observer: Weak<dyn Observer<FdEvents>>) {
        self.subject.register_observer(observer);
    }

    pub fn unregister_observer(&self, observer: Weak<dyn Observer<FdEvents>>) {
        self.subject.unregister_observer(observer);
    }

    fn notify_close_fd_event(&self, fd: FileDescripter) {
        let events = FdEvents::Close(fd);
        self.subject.notify_observers(&events);
    }
}

impl Clone for FileTable {
    fn clone(&self) -> Self {
        Self {
            table: self.table.clone(),
            subject: Subject::new(),
        }
    }
}

impl Drop for FileTable {
    fn drop(&mut self) {
        let events = FdEvents::DropFileTable;
        self.subject.notify_observers(&events);
    }
}

#[derive(Copy, Clone)]
pub enum FdEvents {
    Close(FileDescripter),
    DropFileTable,
}

impl Events for FdEvents {}
