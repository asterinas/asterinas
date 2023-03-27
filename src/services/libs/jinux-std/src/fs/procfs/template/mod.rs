use crate::fs::utils::{FileSystem, Metadata};
use crate::prelude::*;

use super::ProcFS;

pub use self::builder::{ProcDirBuilder, ProcFileBuilder, ProcSymBuilder};
pub use self::dir::{DirOps, ProcDir};
pub use self::file::FileOps;
pub use self::sym::SymOps;

mod builder;
mod dir;
mod file;
mod sym;

struct ProcInodeInfo {
    metadata: Metadata,
    fs: Weak<dyn FileSystem>,
    is_volatile: bool,
}

impl ProcInodeInfo {
    pub fn new(metadata: Metadata, fs: Weak<dyn FileSystem>, is_volatile: bool) -> Self {
        Self {
            metadata,
            fs,
            is_volatile,
        }
    }

    pub fn fs(&self) -> &Weak<dyn FileSystem> {
        &self.fs
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn is_volatile(&self) -> bool {
        self.is_volatile
    }
}
