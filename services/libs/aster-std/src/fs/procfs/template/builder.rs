// SPDX-License-Identifier: MPL-2.0

use crate::fs::utils::{FileSystem, Inode};
use crate::prelude::*;

use super::{
    dir::{DirOps, ProcDir},
    file::{FileOps, ProcFile},
    sym::{ProcSym, SymOps},
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

    pub fn fs(self, fs: Arc<dyn FileSystem>) -> Self {
        self.optional_builder(|ob| ob.fs(fs))
    }

    pub fn volatile(self) -> Self {
        self.optional_builder(|ob| ob.volatile())
    }

    pub fn build(mut self) -> Result<Arc<ProcDir<O>>> {
        let (fs, parent, is_volatile) = self.optional_builder.take().unwrap().build()?;
        Ok(ProcDir::new(self.dir, fs, parent, is_volatile))
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
        let (fs, _, is_volatile) = self.optional_builder.take().unwrap().build()?;
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
        let (fs, _, is_volatile) = self.optional_builder.take().unwrap().build()?;
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
    fs: Option<Arc<dyn FileSystem>>,
    is_volatile: bool,
}

impl OptionalBuilder {
    pub fn parent(mut self, parent: Weak<dyn Inode>) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn fs(mut self, fs: Arc<dyn FileSystem>) -> Self {
        self.fs = Some(fs);
        self
    }

    pub fn volatile(mut self) -> Self {
        self.is_volatile = true;
        self
    }

    #[allow(clippy::type_complexity)]
    pub fn build(self) -> Result<(Arc<dyn FileSystem>, Option<Weak<dyn Inode>>, bool)> {
        if self.parent.is_none() && self.fs.is_none() {
            return_errno_with_message!(Errno::EINVAL, "must have parent or fs");
        }
        let fs = self
            .fs
            .unwrap_or_else(|| self.parent.as_ref().unwrap().upgrade().unwrap().fs());

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

        Ok((fs, self.parent, is_volatile))
    }
}
