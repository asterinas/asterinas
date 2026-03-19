// SPDX-License-Identifier: MPL-2.0

use super::{
    dir::{DirOps, ProcDir},
    file::{FileOps, ProcFile},
    sym::{ProcSym, SymOps},
};
use crate::{
    fs::{
        file::InodeMode,
        vfs::{file_system::FileSystem, inode::Inode},
    },
    prelude::*,
};

pub struct ProcDirBuilder<O: DirOps> {
    // Mandatory field
    dir: O,
    mode: InodeMode,
    // Optional fields
    optional_builder: Option<OptionalBuilder>,
}

impl<O: DirOps> ProcDirBuilder<O> {
    /// Creates a builder for a procfs directory inode.
    pub fn new(dir: O, mode: InodeMode) -> Self {
        let optional_builder = OptionalBuilder::new();
        Self {
            dir,
            mode,
            optional_builder: Some(optional_builder),
        }
    }

    /// Sets the parent inode of the directory.
    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    /// Marks this directory entry as requiring revalidation in its parent directory.
    pub fn need_revalidation(self) -> Self {
        self.optional_builder(|ob| ob.need_revalidation())
    }

    /// Marks cached negative child entries under this directory as requiring revalidation.
    pub fn need_neg_child_revalidation(self) -> Self {
        self.optional_builder(|ob| ob.need_neg_child_revalidation())
    }

    /// Builds the procfs directory inode.
    pub fn build(mut self) -> Result<Arc<ProcDir<O>>> {
        let (fs, parent, ino, need_revalidation, need_neg_child_revalidation) =
            self.optional_builder.take().unwrap().build()?;
        Ok(ProcDir::new(
            self.dir,
            fs,
            parent,
            ino,
            need_revalidation,
            need_neg_child_revalidation,
            self.mode,
        ))
    }

    fn optional_builder<F>(mut self, f: F) -> Self
    where
        F: FnOnce(OptionalBuilder) -> OptionalBuilder,
    {
        let optional_builder = self.optional_builder.take().unwrap();
        self.optional_builder = Some(f(optional_builder));
        self
    }
}

pub struct ProcFileBuilder<O: FileOps> {
    // Mandatory field
    file: O,
    mode: InodeMode,
    // Optional fields
    optional_builder: Option<OptionalBuilder>,
}

impl<O: FileOps> ProcFileBuilder<O> {
    /// Creates a builder for a procfs regular file inode.
    pub fn new(file: O, mode: InodeMode) -> Self {
        let optional_builder = OptionalBuilder::new();
        Self {
            file,
            mode,
            optional_builder: Some(optional_builder),
        }
    }

    /// Sets the parent inode of the file.
    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    /// Marks the file entry as requiring dentry revalidation in its parent directory.
    pub fn need_revalidation(self) -> Self {
        self.optional_builder(|ob| ob.need_revalidation())
    }

    /// Builds the procfs file inode.
    pub fn build(mut self) -> Result<Arc<ProcFile<O>>> {
        let (fs, _, _, need_revalidation, _) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcFile::new(self.file, fs, need_revalidation, self.mode))
    }

    fn optional_builder<F>(mut self, f: F) -> Self
    where
        F: FnOnce(OptionalBuilder) -> OptionalBuilder,
    {
        let optional_builder = self.optional_builder.take().unwrap();
        self.optional_builder = Some(f(optional_builder));
        self
    }
}

pub struct ProcSymBuilder<O: SymOps> {
    // Mandatory field
    sym: O,
    mode: InodeMode,
    // Optional fields
    optional_builder: Option<OptionalBuilder>,
}

impl<O: SymOps> ProcSymBuilder<O> {
    /// Creates a builder for a procfs symbolic-link inode.
    pub fn new(sym: O, mode: InodeMode) -> Self {
        let optional_builder = OptionalBuilder::new();
        Self {
            sym,
            mode,
            optional_builder: Some(optional_builder),
        }
    }

    /// Sets the parent inode of the symbolic link.
    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    /// Marks the symbolic-link entry as requiring dentry revalidation in its parent directory.
    pub fn need_revalidation(self) -> Self {
        self.optional_builder(|ob| ob.need_revalidation())
    }

    /// Builds the procfs symbolic-link inode.
    pub fn build(mut self) -> Result<Arc<ProcSym<O>>> {
        let (fs, _, _, need_revalidation, _) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcSym::new(self.sym, fs, need_revalidation, self.mode))
    }

    fn optional_builder<F>(mut self, f: F) -> Self
    where
        F: FnOnce(OptionalBuilder) -> OptionalBuilder,
    {
        let optional_builder = self.optional_builder.take().unwrap();
        self.optional_builder = Some(f(optional_builder));
        self
    }
}

struct OptionalBuilder {
    parent: Option<Weak<dyn Inode>>,
    fs: Option<Weak<dyn FileSystem>>,
    ino: Option<u64>,
    need_revalidation: bool,
    need_neg_child_revalidation: bool,
}

impl OptionalBuilder {
    fn new() -> Self {
        Self {
            parent: None,
            fs: None,
            ino: None,
            need_revalidation: false,
            need_neg_child_revalidation: false,
        }
    }

    pub fn parent(mut self, parent: Weak<dyn Inode>) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn need_revalidation(mut self) -> Self {
        self.need_revalidation = true;
        self
    }

    pub fn need_neg_child_revalidation(mut self) -> Self {
        self.need_neg_child_revalidation = true;
        self
    }

    #[expect(clippy::type_complexity)]
    pub fn build(
        self,
    ) -> Result<(
        Weak<dyn FileSystem>,
        Option<Weak<dyn Inode>>,
        Option<u64>,
        bool,
        bool,
    )> {
        if self.parent.is_none() && self.fs.is_none() {
            return_errno_with_message!(Errno::EINVAL, "must have parent or fs");
        }
        let fs = self.fs.unwrap_or_else(|| {
            Arc::downgrade(&self.parent.as_ref().unwrap().upgrade().unwrap().fs())
        });

        Ok((
            fs,
            self.parent,
            self.ino,
            self.need_revalidation,
            self.need_neg_child_revalidation,
        ))
    }
}
