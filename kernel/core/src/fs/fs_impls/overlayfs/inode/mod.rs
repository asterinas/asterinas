// SPDX-License-Identifier: MPL-2.0

//! The logical overlay inode and its VFS trait surface.
//!
//! [`OverlayInode`] is the published logical inode shared by every name bound
//! to the same overlay object. It owns the per-object real-object facts, the
//! per-inode transaction lock, the one-way name-taken latch, and the
//! precomputed published identity.
//!
//! # Module map
//!
//! | Submodule | Responsibility |
//! |---|---|
//! | [`copyup`] | the per-object promotion rounds and the temp each round publishes |
//! | [`data`] | the data path: read, write, seek, resize, sync, and the page cache |
//! | [`dir`] | the create, link, remove, and rename namespace mutations, and the whiteout publication |
//! | [`identity`] | the dev/ino identity translation and the durable origin record |
//! | [`inode_cache`] | the mount-wide real-id inode reuse cache |
//! | [`lookup`] | upper-first name resolution and inode projection |
//! | [`metadata`] | the metadata setters |
//! | [`open`] | the open entry and the per-open directory handle |
//! | [`readdir`] | the merged-directory merge and its snapshot construction |
//! | [`xattr`] | the private-record and passthrough name paths |
//!
//! # Locking
//!
//! [`OverlayInode::lock`] is the per-inode transaction lock that serializes one
//! object's mutations. For a directory its payload is that directory's readdir
//! snapshot — a [`readdir::ReaddirCache`], the merged entry table one merge
//! produced; a non-directory uses the lock as a plain serialization token. The
//! lock also serializes an object's promotion against the removal or
//! displacement of its name.
//! [`OverlayInode::append_write`] holds this lock across the underlying `size()` +
//! `write_at` so concurrent appends serialize on the post-write size.
//!
//! The name-taken latch is written — by a removal, or by a rename that displaces
//! the name — and read only inside that object's transaction lock; that lock is
//! what orders a name removal against a promotion round, so the latch needs no
//! ordering of its own.
//!
//! Two overlay-internal locks exist: the per-inode transaction lock and the
//! [`InodeCache`] guard. A per-open snapshot slot sits outside both of them and
//! is the outer lock of its pair: the directory transaction lock is taken and
//! released inside its guard.

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

use spin::Once;

pub(super) use self::{
    dir::whiteout::WhiteoutCache, identity::IdentityPolicy, inode_cache::InodeCache,
    xattr::OverlayXattrType,
};
use self::{
    identity::ObjectVisibleId,
    lookup::{Lookup, NegativeLookup},
    readdir::{OverlayInodeLockGuard, OverlayInodeLockPayload},
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
    this: Weak<OverlayInode>,
}

impl OverlayInode {
    /// Two directories of one mount publish the same pair exactly when they are one real directory.
    fn same_directory_as(&self, other: &OverlayInode) -> bool {
        debug_assert!(self.type_().is_directory() && other.type_().is_directory());
        self.object_id == other.object_id
    }

    /// Returns the read-only take point: the real object this logical object currently shows.
    ///
    /// It is the upper when there is one, else the topmost lower, and reads go through the returned
    /// object's real inode. The object may be a lower: handing it to a write method breaks the write
    /// paths' discipline, which the type does not prevent.
    fn real_object(&self) -> &RealObject {
        match self.upper.get() {
            Some(upper) => upper,
            // A real-object stack is never empty, so a lower-only object has a topmost lower.
            None => self
                .lowers
                .first()
                .expect("a real-object stack is never empty"),
        }
    }

    /// Returns the real object a write may land on.
    ///
    /// Refuses an effectively read-only mount, returns the receiver's own upper when it has one, and
    /// otherwise promotes the receiver into `copyup_dentry` first — so the answer is always an upper.
    fn writable_real_object(&self, copyup_dentry: &Dentry) -> Result<&RealObject> {
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

    /// Returns the receiver's own upper real object.
    ///
    /// The caller must establish that the receiver has an upper; a write path that has not promoted the
    /// receiver should enter through `writable_real_object` instead.
    fn writable_upper(&self) -> &RealObject {
        self.upper
            .get()
            .expect("an overlay object on a write path has an upper real object")
    }

    /// Takes the per-inode transaction lock.
    fn lock(&self) -> OverlayInodeLockGuard<'_> {
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
}

/// One create-family request; `mode` rides the op and the borrowed fields pass the VFS entry on.
#[derive(Clone, Copy)]
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
    fn general(kind: InodeType, mode: InodeMode) -> Self {
        debug_assert!(
            kind != InodeType::SymLink,
            "overlay symlinks are created only through create_symlink",
        );
        Self::General { kind, mode }
    }

    fn symlink(target: &'a str, mode: InodeMode) -> Self {
        Self::Symlink { target, mode }
    }

    fn mknod(node: &'a MknodType, mode: InodeMode) -> Result<Self> {
        // The read side reads a raw `0:0` char device back as a whiteout, so a caller must not
        // be able to create one and forge the name-level shadow this mount publishes itself.
        if matches!(node, MknodType::CharDevice(0)) {
            return_errno_with_message!(
                Errno::EPERM,
                "a raw 0:0 whiteout char device must not be user-creatable"
            );
        }
        Ok(Self::Mknod { mode, node })
    }

    /// Returns the inode type this request creates.
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

    /// Runs the one real create call this request selects, in `dir`, under `name`.
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
        self.create_impl(self_dentry, name, CreateOp::general(type_, mode))
    }

    fn create_symlink(
        &self,
        self_dentry: &Dentry,
        name: &str,
        target: &str,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        self.create_impl(self_dentry, name, CreateOp::symlink(target, mode))
    }

    fn mknod(
        &self,
        self_dentry: &Dentry,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        self.create_impl(self_dentry, name, CreateOp::mknod(&type_, mode)?)
    }

    fn link(&self, self_dentry: &Dentry, old_dentry: &Dentry, name: &str) -> Result<()> {
        self.link_impl(self_dentry, old_dentry, name)
    }

    fn unlink(&self, child_dentry: &Dentry) -> Result<()> {
        self.unlink_impl(child_dentry)
    }

    fn rmdir(&self, child_dentry: &Dentry) -> Result<()> {
        self.rmdir_impl(child_dentry)
    }

    fn rename(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        replaced_inode: Option<&Arc<dyn Inode>>,
        mode: RenameMode,
    ) -> Result<()> {
        self.rename_impl(
            old_child_dentry,
            new_dir_dentry,
            new_name,
            replaced_inode,
            mode,
        )
    }
}
