// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

pub(super) use self::{
    builder::{ProcDirBuilder, ProcFileBuilder, ProcSymBuilder},
    dir::{DirOps, ProcDir, lookup_child_from_table, populate_children_from_table},
    file::{FileOps, FileOpsByHandle, ProcFile},
    sym::{ProcSym, SymOps},
};
use super::{BLOCK_SIZE, ProcFs};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Metadata},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

mod builder;
mod dir;
mod file;
mod sym;

struct Common {
    metadata: RwLock<Metadata>,
    extension: Extension,
    fs: Weak<dyn FileSystem>,
    is_volatile: bool,
}

impl Common {
    fn new(metadata: Metadata, fs: Weak<dyn FileSystem>, is_volatile: bool) -> Self {
        Self {
            metadata: RwLock::new(metadata),
            extension: Extension::new(),
            fs,
            is_volatile,
        }
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    fn ino(&self) -> u64 {
        self.metadata.read().ino
    }

    fn type_(&self) -> InodeType {
        self.metadata.read().type_
    }

    fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn atime(&self) -> Duration {
        self.metadata.read().last_access_at
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.write().last_access_at = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata.read().last_modify_at
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.write().last_modify_at = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata.read().last_meta_change_at
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.write().last_meta_change_at = time;
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.read().mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.read().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.read().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        Ok(())
    }

    fn is_volatile(&self) -> bool {
        self.is_volatile
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}
