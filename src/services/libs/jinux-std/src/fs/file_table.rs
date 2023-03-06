use crate::prelude::*;

use super::{
    file_handle::FileHandle,
    stdio::{Stderr, Stdin, Stdout, FD_STDERR, FD_STDIN, FD_STDOUT},
};

pub type FileDescripter = i32;

#[derive(Clone)]
pub struct FileTable {
    table: BTreeMap<FileDescripter, FileHandle>,
}

impl FileTable {
    pub fn new() -> Self {
        Self {
            table: BTreeMap::new(),
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
        Self { table }
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
        self.table.insert(fd, item)
    }

    pub fn close_file(&mut self, fd: FileDescripter) -> Option<FileHandle> {
        self.table.remove(&fd)
    }

    pub fn get_file(&self, fd: FileDescripter) -> Result<&FileHandle> {
        self.table
            .get(&fd)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }
}
