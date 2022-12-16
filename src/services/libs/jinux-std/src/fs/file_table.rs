use crate::prelude::*;

use super::{
    file::{File, FileDescripter},
    stdio::{Stderr, Stdin, Stdout, FD_STDERR, FD_STDIN, FD_STDOUT},
};

#[derive(Clone)]
pub struct FileTable {
    table: BTreeMap<FileDescripter, Arc<dyn File>>,
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
        table.insert(FD_STDIN, Arc::new(stdin) as Arc<dyn File>);
        table.insert(FD_STDOUT, Arc::new(stdout) as Arc<dyn File>);
        table.insert(FD_STDERR, Arc::new(stderr) as Arc<dyn File>);
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

    pub fn insert(&mut self, item: Arc<dyn File>) -> FileDescripter {
        let fd = self.max_fd() + 1;
        self.table.insert(fd, item);
        fd
    }

    pub fn close_file(&mut self, fd: FileDescripter) {
        self.table.remove(&fd);
    }

    pub fn get_file(&self, fd: FileDescripter) -> Result<&Arc<dyn File>> {
        self.table
            .get(&fd)
            .ok_or(Error::with_message(Errno::EBADF, "fd not exits"))
    }
}
