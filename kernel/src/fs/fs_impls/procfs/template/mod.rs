// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

pub(super) use self::{
    dir::{
        ListedEntry, ProcDir, ProcDirOps, ReaddirEntry, StaticDirEntry, keyed_readdir_entries,
        listed_entries_from_table, lookup_child_from_table, sequential_readdir_entries,
        visit_listed_entries, visit_readdir_entries,
    },
    file::{ProcFile, ProcFileOps, ProcFileOpsByHandle, read_i32_from},
    sym::{ProcSym, ProcSymOps},
};
use crate::{
    fs::{
        file::InodeMode,
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Metadata},
        },
    },
    prelude::*,
    process::{Gid, Uid, posix_thread::AsPosixThread},
    thread::Thread,
};

mod dir;
mod file;
mod sym;

/// Shared procfs inode state.
///
/// FIXME: Procfs permissions should be checked during each operation, not by relying on mutable
/// inode ownership. See Linux comment:
/// <https://elixir.bootlin.com/linux/v6.13/source/fs/proc/base.c#L107>.
struct Common {
    metadata: RwLock<Metadata>,
    extension: Extension,
    fs: Weak<dyn FileSystem>,
}

impl Common {
    fn new(metadata: Metadata, fs: Weak<dyn FileSystem>) -> Self {
        Self {
            metadata: RwLock::new(metadata),
            extension: Extension::new(),
            fs,
        }
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    fn metadata_with_owner(&self, owner_thread: Option<Arc<Thread>>) -> Metadata {
        let Some(owner_thread) = owner_thread else {
            return self.metadata();
        };

        let credentials = owner_thread.as_posix_thread().unwrap().credentials();
        let mut metadata = self.metadata.write();
        // Cache the dynamic owner into the metadata so that if the thread
        // later exits, subsequent calls fall back to the last known owner
        // instead of the root user. This is a best-effort attempt to align
        // with Linux behavior. See:
        // <https://github.com/asterinas/asterinas/pull/3164#discussion_r3212307770>.
        metadata.uid = credentials.euid();
        metadata.gid = credentials.egid();
        *metadata
    }

    fn ino(&self) -> u64 {
        self.metadata.read().ino
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

    fn extension(&self) -> &Extension {
        &self.extension
    }
}
