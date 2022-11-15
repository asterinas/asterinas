use crate::prelude::*;

use super::{
    file::{File, FileDescripter},
    stdio::{Stderr, Stdin, Stdout, FD_STDERR, FD_STDIN, FD_STDOUT},
};

#[derive(Debug, Clone)]
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
        table.insert(FD_STDIN, Arc::new(Stdin) as Arc<dyn File>);
        table.insert(FD_STDOUT, Arc::new(Stdout) as Arc<dyn File>);
        table.insert(FD_STDERR, Arc::new(Stderr) as Arc<dyn File>);
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

    pub fn get_file(&self, fd: FileDescripter) -> Option<&Arc<dyn File>> {
        self.table.get(&fd)
    }
}
