// SPDX-License-Identifier: MPL-2.0

//! The logical overlay inode and its VFS trait surface.
//!
//! [`OverlayInode`] is the published logical inode shared by every name bound
//! to the same overlay object. It owns the per-object real-object facts, the
//! per-inode transaction lock, the one-way name-taken latch, and the
//! precomputed published identity.
//!
//! # Module structure
//!
//! | Submodule | Responsibility |
//! |---|---|
//! | [`copyup`] | Copy-up promotion of lower-backed objects to the upper layer. |
//! | [`data`] | Data-path read/write through the two take points (read, write, resize, sync). |
//! | [`dir`] | Directory namespace mutations and whiteout publication. |
//! | [`identity`] | Dev/ino identity translation and the origin record. |
//! | [`inode_cache`] | The mount-wide real-id inode reuse cache. |
//! | [`lookup`] | Upper-first name resolution and inode projection. |
//! | [`metadata`] | The six metadata setters and their ownership gate. |
//! | [`open`] | The open entry through the writable take point, and the per-open handle. |
//! | [`readdir`] | The merge and snapshot construction; enumeration is consumed by [`open`]. |
//! | [`xattr`] | The xattr private-record and passthrough paths. |
//!
//! # Locking
//!
//! `lock` is the per-inode transaction lock. For a directory, its payload is the
//! current snapshot slot: it holds `None` while no snapshot is built or after one
//! is invalidated, and otherwise the immutable merge result that every reader
//! shares through `Arc`. A non-directory keeps that slot empty and uses the lock
//! as a plain serialization token; a per-open directory handle owns a separate
//! slot for the snapshot that one open file iterates, and it reads this per-inode
//! slot only to adopt the current snapshot or to rebuild one.
//! [`OverlayInode::append_write`] holds this lock across the underlying `size()` +
//! `write_at` so concurrent appends serialize on the post-write size.
//!
//! A copy-up collects its chain with no overlay lock held, then promotes one
//! object per round: each round takes the publication parent's transaction
//! lock before the promoted object's — the same ancestor-to-descendant
//! direction as the removal and rename target-object locks — releases both
//! before the next round, and reads its own two latches under both locks to
//! learn whether its coordinate is still valid. The one `InodeCache` guard a
//! round takes is the commit tail's rekey of the promoted instance, taken after
//! that instance's facts are replaced and below both transaction locks, so the
//! cache stays a leaf domain.
//!
//! The name-taken latch is read and written only inside its object's
//! transaction lock; that lock is what orders a name removal against a
//! promotion round, so the latch needs no ordering of its own and no lock-free
//! read of it may be introduced.
//!
//! No other lock domains exist: the per-inode transaction lock and the
//! `InodeCache` guard are the only overlay-internal locks. A per-open
//! snapshot slot sits outside both of them and is the outer lock of its pair:
//! the directory transaction lock is taken and released inside its guard.

mod copyup;
mod data;
mod dir;
mod identity;
mod inode_cache;
mod lookup;
mod metadata;
mod open;
mod readdir;
mod xattr;

use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) use copyup::workdir::workdir_temp_name;
use spin::Once;

pub(super) use self::{
    dir::whiteout::WhiteoutCache, identity::IdentityPolicy, inode_cache::InodeCache,
    xattr::OverlayXattrType,
};
use self::{
    identity::ObjectVisibleId,
    lookup::{Lookup, NegativeLookup, is_opaque_directory, is_whiteout_inode},
    readdir::OverlayInodeLockPayload,
};
use crate::{
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, StatusFlags, SyncMode},
        fs_impls::overlayfs::{fs::OverlayFs, real::RealObject},
        vfs::{
            file_system::FileSystem,
            inode::{
                Extension, FallocMode, FileOps, Inode, Metadata, MknodType, RenameMode,
                SymbolicLink,
            },
            path::Dentry,
            xattr::{XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::Vmo,
};

pub(super) struct OverlayInode {
    fs: Weak<OverlayFs>,
    /// A directory keeps its whole merge stack; a non-directory only its visible topmost lower.
    lowers: Vec<RealObject>,
    upper: Once<RealObject>,
    /// One-way name-taken latch, set under this object's lock: its name binding was taken away.
    name_taken: AtomicBool,
    object_id: ObjectVisibleId,
    /// The current readdir snapshot slot; `None` means it is not built or was invalidated.
    lock: Mutex<OverlayInodeLockPayload>,
    extension: Extension,
    /// The only self-reference; initialized by `Arc::new_cyclic`.
    this: Weak<OverlayInode>,
}

impl OverlayInode {
    /// Precondition: both are directories of one mount — the only caller's move guarantees it.
    /// Two directories of one mount publish the same pair exactly when they are one real directory.
    pub(super) fn same_directory_as(&self, other: &OverlayInode) -> bool {
        debug_assert!(self.type_().is_directory() && other.type_().is_directory());
        self.object_id == other.object_id
    }

    /// The read-only take point: the real object this logical object currently shows — the upper
    /// when it has one, else its topmost lower. Reads go through the returned object's real inode.
    /// The object it returns may be a lower: handing it to a write method is a discipline violation
    /// of the write paths, which the type does not prevent.
    pub(super) fn real_object(&self) -> &RealObject {
        match self.upper.get() {
            Some(upper) => upper,
            // A real-object stack is never empty, so a lower-only object has a topmost lower.
            None => self
                .lowers
                .first()
                .expect("a real-object stack is never empty"),
        }
    }

    /// The writable take point: the read-only gate, an already-promoted receiver's own upper, the
    /// promotion, and the upper writes go through.
    pub(super) fn writable_real_object(&self, copyup_dentry: &Dentry) -> Result<&RealObject> {
        let fs = self.fs_arc();
        if fs.policy().is_effective_read_only() {
            return_errno_with_message!(Errno::EROFS, "the overlay mount is read-only");
        }
        if let Some(upper) = self.upper.get() {
            return Ok(upper);
        }
        fs.copy_up_at(copyup_dentry)?;
        Ok(self.writable_upper())
    }

    /// Sets the name-taken latch; the caller must hold this object's transaction lock.
    fn mark_name_taken(&self) {
        // Only the transaction lock orders this against `is_name_taken`: no lock-free read exists.
        self.name_taken.store(true, Ordering::Relaxed);
    }

    /// Reads the name-taken latch; the caller must hold this object's transaction lock.
    fn is_name_taken(&self) -> bool {
        self.name_taken.load(Ordering::Relaxed)
    }

    /// The receiver's own upper real object: every caller sits on a write path, so a missing upper
    /// breaks an invariant rather than answering a lower-only question.
    fn writable_upper(&self) -> &RealObject {
        self.upper
            .get()
            .expect("an overlay object on a write path has an upper real object")
    }

    /// The per-inode transaction lock; non-directories use it as a plain token.
    fn lock(&self) -> MutexGuard<'_, OverlayInodeLockPayload> {
        Mutex::lock(&self.lock)
    }

    fn fs_arc(&self) -> Arc<OverlayFs> {
        self.fs
            .upgrade()
            .expect("the owning fs outlives every overlay inode")
    }

    fn self_arc(&self) -> Arc<OverlayInode> {
        self.this
            .upgrade()
            .expect("the overlay inode is still owned by an Arc")
    }

    fn append_write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize> {
        let _guard = self.lock();
        let real = self.writable_upper();
        let offset = real.real_inode().size();
        real.real_inode().write_at(offset, reader, status_flags)
    }
}

/// One create-family request; `mode` rides the op and the borrowed fields pass the VFS entry on.
enum CreateOp<'a> {
    General {
        kind: InodeType,
        mode: InodeMode,
    },
    Symlink {
        target: &'a str,
        mode: InodeMode,
    },
    Mknod {
        mode: InodeMode,
        node: &'a MknodType,
    },
}

impl<'a> CreateOp<'a> {
    /// The VFS `create` entry; the VFS routes symlink creation only through `create_symlink`.
    fn general(kind: InodeType, mode: InodeMode) -> Self {
        debug_assert!(
            kind != InodeType::SymLink,
            "overlay symlinks are created only through create_symlink",
        );
        Self::General { kind, mode }
    }

    /// The VFS `create_symlink` entry; the target is atomic with creation.
    fn symlink(target: &'a str, mode: InodeMode) -> Self {
        Self::Symlink { target, mode }
    }

    /// The VFS `mknod` entry; a raw `0:0` whiteout device is not user-creatable.
    fn mknod(node: &'a MknodType, mode: InodeMode) -> Self {
        debug_assert!(
            !matches!(node, MknodType::CharDevice(0)),
            "a raw 0:0 whiteout char device must not be user-creatable",
        );
        Self::Mknod { mode, node }
    }

    /// The single type derivation for every create-family entry.
    fn object_type(&self) -> InodeType {
        match self {
            Self::General { kind, .. } => *kind,
            Self::Symlink { .. } => InodeType::SymLink,
            Self::Mknod { node, .. } => match *node {
                MknodType::NamedPipe => InodeType::NamedPipe,
                MknodType::CharDevice(_) => InodeType::CharDevice,
                MknodType::BlockDevice(_) => InodeType::BlockDevice,
            },
        }
    }

    /// The single create entry: the destination is a real dentry of the writable side.
    fn create_child_in(&self, dir: &Dentry, name: &str) -> Result<Arc<Dentry>> {
        let dir = dir.as_dir_dentry_or_err()?;
        match self {
            Self::General { kind, mode } => {
                dir.create_child(name, || dir.inode().create(&dir, name, *kind, *mode))
            }
            Self::Symlink { target, mode } => dir.create_child(name, || {
                dir.inode().create_symlink(&dir, name, target, *mode)
            }),
            Self::Mknod { mode, node } => dir.mknod(
                name,
                *mode,
                match *node {
                    MknodType::NamedPipe => MknodType::NamedPipe,
                    MknodType::CharDevice(device) => MknodType::CharDevice(*device),
                    MknodType::BlockDevice(device) => MknodType::BlockDevice(*device),
                },
            ),
        }
    }
}

impl FileOps for OverlayInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.read_at_impl(offset, writer, status_flags)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.write_at_impl(offset, reader, status_flags)
    }
}

impl Inode for OverlayInode {
    fn size(&self) -> usize {
        self.real_object().real_inode().size()
    }

    fn metadata(&self) -> Result<Metadata> {
        let mut metadata = self.real_object().real_inode().metadata()?;
        metadata.ino = self.object_id.ino;
        metadata.container_dev_id = self.object_id.dev;
        Ok(metadata)
    }

    fn ino(&self) -> u64 {
        self.object_id.ino
    }

    fn type_(&self) -> InodeType {
        self.real_object().real_inode().type_()
    }

    fn mode(&self) -> Result<InodeMode> {
        self.real_object().real_inode().mode()
    }

    fn owner(&self) -> Result<Uid> {
        self.real_object().real_inode().owner()
    }

    fn group(&self) -> Result<Gid> {
        self.real_object().real_inode().group()
    }

    fn atime(&self) -> Duration {
        self.real_object().real_inode().atime()
    }

    fn mtime(&self) -> Duration {
        self.real_object().real_inode().mtime()
    }

    fn ctime(&self) -> Duration {
        self.real_object().real_inode().ctime()
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        // The guard is still taken here: it orders this lookup against this directory's mutations.
        let _dir_guard = self.lock();
        // The directory bit comes from the real object alone; the payload says nothing about it.
        if !self.type_().is_directory() {
            return Err(Error::with_message(
                Errno::ENOTDIR,
                "lookup is supported on overlay directories only",
            ));
        }
        let fs = self.fs.upgrade().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the overlay mount is no longer alive")
        })?;
        match fs.lookup(self, name)? {
            Lookup::Positive(inode) => Ok(inode),
            Lookup::Negative(_) => Err(Error::new(Errno::ENOENT)),
        }
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs_arc()
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }

    fn open(
        &self,
        self_dentry: &Dentry,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        self.open_impl(self_dentry, access_mode, status_flags)
    }

    fn seek_end(&self) -> Option<usize> {
        self.seek_end_impl()
    }

    fn resize(&self, self_dentry: &Dentry, new_size: usize) -> Result<()> {
        self.resize_impl(self_dentry, new_size)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.fallocate_impl(mode, offset, len)
    }

    fn sync(&self, mode: SyncMode) -> Result<()> {
        self.sync_impl(mode)
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link_impl()
    }

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        self.page_cache_impl()
    }

    fn set_mode(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()> {
        self.set_mode_impl(self_dentry, mode)
    }

    fn set_owner(&self, self_dentry: &Dentry, uid: Uid) -> Result<()> {
        self.set_owner_impl(self_dentry, uid)
    }

    fn set_group(&self, self_dentry: &Dentry, gid: Gid) -> Result<()> {
        self.set_group_impl(self_dentry, gid)
    }

    fn set_atime(&self, self_dentry: &Dentry, time: Duration) {
        self.set_atime_impl(self_dentry, time)
    }

    fn set_mtime(&self, self_dentry: &Dentry, time: Duration) {
        self.set_mtime_impl(self_dentry, time)
    }

    fn set_ctime(&self, self_dentry: &Dentry, time: Duration) {
        self.set_ctime_impl(self_dentry, time)
    }

    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        self.get_xattr_impl(name, value_writer)
    }

    fn set_xattr(
        &self,
        self_dentry: &Dentry,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.set_xattr_impl(self_dentry, name, value_reader, flags)
    }

    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        self.list_xattr_impl(namespace, list_writer)
    }

    fn remove_xattr(&self, self_dentry: &Dentry, name: XattrName) -> Result<()> {
        self.remove_xattr_impl(self_dentry, name)
    }

    fn create(
        &self,
        self_dentry: &Dentry,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        let op = CreateOp::general(type_, mode);
        self.create_impl(self_dentry, name, &op)
    }

    fn create_symlink(
        &self,
        self_dentry: &Dentry,
        name: &str,
        target: &str,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        self.create_impl(self_dentry, name, &CreateOp::symlink(target, mode))
    }

    fn mknod(
        &self,
        self_dentry: &Dentry,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        let op = CreateOp::mknod(&type_, mode);
        self.create_impl(self_dentry, name, &op)
    }

    fn link(&self, self_dentry: &Dentry, old_dentry: &Dentry, name: &str) -> Result<()> {
        self.link_impl(self_dentry, old_dentry, name)
    }

    fn unlink(&self, child_dentry: &Dentry) -> Result<()> {
        let name = child_dentry.name();
        self.unlink_impl(child_dentry, name)
    }

    fn rmdir(&self, child_dentry: &Dentry) -> Result<()> {
        let name = child_dentry.name();
        self.rmdir_impl(child_dentry, name)
    }

    fn rename(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        target_dentry: Option<&Dentry>,
        mode: RenameMode,
    ) -> Result<()> {
        self.rename_impl(
            old_child_dentry,
            new_dir_dentry,
            new_name,
            target_dentry,
            mode,
        )
    }
}
