// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use super::InodeType;
use crate::{fs::path::is_dot_or_dotdot, prelude::*};

/// A visitor for dir entries.
pub trait DirentVisitor {
    /// Visit a dir entry.
    ///
    /// If the visitor succeeds in visiting the given inode, an `Ok(())` is returned;
    /// Otherwise, an error is returned. Different implementations for `DirentVisitor`
    /// may choose to report errors for different reasons. Regardless of the exact
    /// errors and reasons, `readdir`-family methods shall stop feeding the visitor
    /// with the next inode as long as an error is returned by the visitor.
    ///
    /// # Example
    ///
    /// `Vec<String>` is implemented as `DirentVisitor` so that the file names
    /// under a dir can be easily collected, which is convenient for testing purposes.
    ///
    /// ```no_run
    /// let mut all_dirents = Vec::new();
    /// let dir_inode = todo!("create an inode");
    /// dir_inode.readdir_at(0, &mut all_dirents).unwrap();
    /// ```
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()>;
}

impl DirentVisitor for Vec<String> {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()> {
        self.push(name.into());
        Ok(())
    }
}

/// Utility to count directory entries, excluding "." and ".."
#[derive(Default)]
pub struct DirentCounter(usize);

impl DirentCounter {
    pub fn new() -> Self {
        Self(0)
    }

    pub fn count(&self) -> usize {
        self.0
    }
}

impl DirentVisitor for DirentCounter {
    fn visit(&mut self, name: &str, _ino: u64, _type: InodeType, _offset: usize) -> Result<()> {
        if !is_dot_or_dotdot(name) {
            self.0 += 1;
        }
        Ok(())
    }
}
