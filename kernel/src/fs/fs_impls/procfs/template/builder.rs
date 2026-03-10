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
    pub fn new(dir: O, mode: InodeMode) -> Self {
        let optional_builder = OptionalBuilder::new();
        Self {
            dir,
            mode,
            optional_builder: Some(optional_builder),
        }
    }

    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    pub fn volatile(self) -> Self {
        self.optional_builder(|ob| ob.volatile())
    }

    pub fn build(mut self) -> Result<Arc<ProcDir<O>>> {
        let (fs, parent, is_volatile) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcDir::new(self.dir, fs, parent, is_volatile, self.mode))
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
    pub fn new(file: O, mode: InodeMode) -> Self {
        let optional_builder = OptionalBuilder::new();
        Self {
            file,
            mode,
            optional_builder: Some(optional_builder),
        }
    }

    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    #[expect(dead_code)]
    pub fn volatile(self) -> Self {
        self.optional_builder(|ob| ob.volatile())
    }

    pub fn build(mut self) -> Result<Arc<ProcFile<O>>> {
        let (fs, _, is_volatile) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcFile::new(self.file, fs, is_volatile, self.mode))
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
    pub fn new(sym: O, mode: InodeMode) -> Self {
        let optional_builder = OptionalBuilder::new();
        Self {
            sym,
            mode,
            optional_builder: Some(optional_builder),
        }
    }

    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    #[expect(dead_code)]
    pub fn volatile(self) -> Self {
        self.optional_builder(|ob| ob.volatile())
    }

    pub fn build(mut self) -> Result<Arc<ProcSym<O>>> {
        let (fs, _, is_volatile) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcSym::new(self.sym, fs, is_volatile, self.mode))
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
    is_volatile: bool,
}

impl OptionalBuilder {
    fn new() -> Self {
        Self {
            parent: None,
            is_volatile: false,
        }
    }

    pub fn parent(mut self, parent: Weak<dyn Inode>) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn volatile(mut self) -> Self {
        self.is_volatile = true;
        self
    }

    #[expect(clippy::type_complexity)]
    pub fn build(self) -> Result<(Weak<dyn FileSystem>, Option<Weak<dyn Inode>>, bool)> {
        let Some(parent) = self.parent else {
            return_errno_with_message!(Errno::EINVAL, "must have parent");
        };
        let parent_inode = parent.upgrade().unwrap();
        let fs = Arc::downgrade(&parent_inode.fs());

        // The volatile property is inherited from parent.
        let is_volatile = {
            let mut is_volatile = self.is_volatile;
            if !parent_inode.is_dentry_cacheable() {
                is_volatile = true;
            }

            is_volatile
        };

        Ok((fs, Some(parent), is_volatile))
    }
}
