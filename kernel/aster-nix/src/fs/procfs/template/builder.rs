// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::{
    dir::{DirOps, ProcDir},
    file::{FileOps, ProcFile},
    sym::{ProcSym, SymOps},
};
use crate::{
    fs::utils::{FileSystem, Inode},
    prelude::*,
};

pub struct ProcDirBuilder<O: DirOps> {
    // Mandatory field
    dir: O,
    // Optional fields
    optional_builder: Option<OptionalBuilder>,
}

impl<O: DirOps> ProcDirBuilder<O> {
    pub fn new(dir: O) -> Self {
        let optional_builder: OptionalBuilder = Default::default();
        Self {
            dir,
            optional_builder: Some(optional_builder),
        }
    }

    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    pub fn fs(self, fs: Weak<dyn FileSystem>) -> Self {
        self.optional_builder(|ob| ob.fs(fs))
    }

    pub fn volatile(self) -> Self {
        self.optional_builder(|ob| ob.volatile())
    }

    pub fn ino(self, ino: u64) -> Self {
        self.optional_builder(|ob| ob.ino(ino))
    }

    pub fn build(mut self) -> Result<Arc<ProcDir<O>>> {
        let (fs, parent, ino, is_volatile) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcDir::new(self.dir, fs, parent, ino, is_volatile))
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
    // Optional fields
    optional_builder: Option<OptionalBuilder>,
}

impl<O: FileOps> ProcFileBuilder<O> {
    pub fn new(file: O) -> Self {
        let optional_builder: OptionalBuilder = Default::default();
        Self {
            file,
            optional_builder: Some(optional_builder),
        }
    }

    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    pub fn volatile(self) -> Self {
        self.optional_builder(|ob| ob.volatile())
    }

    pub fn build(mut self) -> Result<Arc<ProcFile<O>>> {
        let (fs, _, _, is_volatile) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcFile::new(self.file, fs, is_volatile))
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
    // Optional fields
    optional_builder: Option<OptionalBuilder>,
}

impl<O: SymOps> ProcSymBuilder<O> {
    pub fn new(sym: O) -> Self {
        let optional_builder: OptionalBuilder = Default::default();
        Self {
            sym,
            optional_builder: Some(optional_builder),
        }
    }

    pub fn parent(self, parent: Weak<dyn Inode>) -> Self {
        self.optional_builder(|ob| ob.parent(parent))
    }

    pub fn volatile(self) -> Self {
        self.optional_builder(|ob| ob.volatile())
    }

    pub fn build(mut self) -> Result<Arc<ProcSym<O>>> {
        let (fs, _, _, is_volatile) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcSym::new(self.sym, fs, is_volatile))
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

#[derive(Default)]
struct OptionalBuilder {
    parent: Option<Weak<dyn Inode>>,
    fs: Option<Weak<dyn FileSystem>>,
    ino: Option<u64>,
    is_volatile: bool,
}

impl OptionalBuilder {
    pub fn parent(mut self, parent: Weak<dyn Inode>) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn fs(mut self, fs: Weak<dyn FileSystem>) -> Self {
        self.fs = Some(fs);
        self
    }

    pub fn ino(mut self, ino: u64) -> Self {
        self.ino = Some(ino);
        self
    }

    pub fn volatile(mut self) -> Self {
        self.is_volatile = true;
        self
    }

    #[allow(clippy::type_complexity)]
    pub fn build(
        self,
    ) -> Result<(
        Weak<dyn FileSystem>,
        Option<Weak<dyn Inode>>,
        Option<u64>,
        bool,
    )> {
        if self.parent.is_none() && self.fs.is_none() {
            return_errno_with_message!(Errno::EINVAL, "must have parent or fs");
        }
        let fs = self.fs.unwrap_or_else(|| {
            Arc::downgrade(&self.parent.as_ref().unwrap().upgrade().unwrap().fs())
        });

        // The volatile property is inherited from parent.
        let is_volatile = {
            let mut is_volatile = self.is_volatile;
            if let Some(parent) = self.parent.as_ref() {
                if !parent.upgrade().unwrap().is_dentry_cacheable() {
                    is_volatile = true;
                }
            }
            is_volatile
        };

        Ok((fs, self.parent, self.ino, is_volatile))
    }
}
