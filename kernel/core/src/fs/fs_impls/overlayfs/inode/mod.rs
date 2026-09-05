// SPDX-License-Identifier: MPL-2.0

//! The logical overlay inode and its VFS trait surface.
//!
//! [`OverlayInode`] is the published logical inode shared by every name bound
//! to the same overlay object. It owns the per-object real-object facts, the
//! per-inode transaction lock, the precomputed published identity, and the
//! copy-up winner/waiter token.
//!
//! # Module structure
//!
//! | Submodule | Responsibility |
//! |---|---|
//! | [`copyup`] | Copy-up promotion of lower-backed objects to the upper layer. |
//! | [`data`] | Data-path delegation to the real authority (read, write, resize, sync). |
//! | [`dir`] | Directory namespace mutations and whiteout publication. |
//! | [`identity`] | Dev/ino identity translation and the lower-id record. |
//! | [`inode_cache`] | The mount-wide real-object-key inode reuse cache. |
//! | [`lookup`] | Upper-first name resolution and inode projection. |
//! | [`metadata`] | The six metadata setters behind the admission pipeline. |
//! | [`permission`] | The two-stage permission admission pipeline. |
//! | [`readdir`] | The merged-directory readdir cache and enumeration. |
//! | [`xattr`] | The xattr private-record and passthrough paths. |
//!
//! # Locking
//!
//! `lock` is the per-inode transaction lock; directories carry a
//! [`ReaddirCache`] in its payload, while non-directories use it as a plain
//! serialization token. [`OverlayInode::append_write`] holds this lock across
//! the underlying `size()` + `write_at` so concurrent appends serialize on the
//! post-write size.
//!
//! The copy-up frame orders its locks strictly: the object's `copyup`
//! winner/waiter mutex, then the publication parent's directory transaction
//! lock, then the `InodeCache` write guard (the innermost leaf). Coordinate
//! extraction precedes every overlay lock: the copy-up publication coordinate
//! is read as `(parent, name)` from the operation's overlay dentry before any
//! overlay lock. No other lock domains exist: the per-inode transaction lock,
//! the `copyup` mutex, and the `InodeCache` write guard are the only
//! overlay-internal locks.

#![short_vis_path::add(overlayfs)]

mod copyup;
mod data;
mod dir;
mod identity;
mod inode_cache;
mod lookup;
mod metadata;
mod permission;
mod readdir;
mod xattr;

use core::time::Duration;

pub(in overlayfs) use copyup::workdir::workdir_temp_name;
use spin::Once;

pub(super) use self::{
    dir::whiteout::WhiteoutCache, identity::IdentityPolicy, inode_cache::InodeCache,
    xattr::OverlayRecordName,
};
use self::{
    identity::ObjectId,
    lookup::{Lookup, NegativeLookup, is_opaque_directory, is_whiteout_inode},
    permission::AccessType,
    readdir::ReaddirCache,
};
use crate::{
    fs::{
        file::{
            AccessMode, InodeMode, InodeType, PerOpenFileOps, Permission, StatusFlags, SyncMode,
        },
        fs_impls::overlayfs::{
            fs::OverlayFs,
            real::{RealObject, RealObjectKey},
        },
        utils::DirentVisitor,
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

/// One create-family request; `mode` rides the op and the borrowed fields pass the VFS entry on.
pub(super) enum CreateOp<'a> {
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
    /// The VFS `create` entry: symlinks are reachable only through `create_symlink`.
    pub(super) fn general(kind: InodeType, mode: InodeMode) -> Result<Self> {
        if kind == InodeType::SymLink {
            return_errno_with_message!(
                Errno::EINVAL,
                "overlay symlinks are created only through create_symlink"
            );
        }
        Ok(Self::General { kind, mode })
    }

    /// The VFS `create_symlink` entry; the target is atomic with creation.
    pub(super) fn symlink(target: &'a str, mode: InodeMode) -> Self {
        Self::Symlink { target, mode }
    }

    /// The VFS `mknod` entry: the raw 0:0 whiteout device must not be user-creatable.
    pub(super) fn mknod(node: &'a MknodType, mode: InodeMode) -> Result<Self> {
        if matches!(node, MknodType::CharDevice(0)) {
            return_errno_with_message!(
                Errno::EPERM,
                "a raw 0:0 whiteout char device must not be user-creatable"
            );
        }
        Ok(Self::Mknod { mode, node })
    }

    /// The single type derivation for every create-family entry.
    pub(super) fn object_type(&self) -> InodeType {
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

    /// Creates the object this op denotes inside `dir`, mapping to one of three real syscalls.
    pub(super) fn create_child_in(&self, dir: &Dentry, name: &str) -> Result<Arc<Dentry>> {
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

pub(super) struct OverlayInode {
    fs: Weak<OverlayFs>,
    lowers: Vec<RealObject>,
    upper: Once<RealObject>,
    object_id: ObjectId,
    lock: Mutex<Option<ReaddirCache>>,
    /// The winner/waiter token serializing concurrent first copy-ups; the only state is `upper`.
    copyup: Mutex<()>,
    extension: Extension,
    /// The only self-reference; initialized by `Arc::new_cyclic`.
    this: Weak<OverlayInode>,
}

impl OverlayInode {
    /// The visible-source key used by rename for the same-parent check.
    fn key(&self, fs: &OverlayFs) -> RealObjectKey {
        fs.real_object_key(self.visible_source())
    }

    fn visible_source(&self) -> &RealObject {
        self.upper.get().unwrap_or_else(|| {
            self.lowers
                .first()
                .expect("a real-object stack is never empty")
        })
    }

    fn object_id(&self) -> ObjectId {
        self.object_id
    }

    /// The upper real parent dentry; the dir family takes this instead of rebuilding a `Path`.
    fn upper_parent_dentry(&self) -> Result<&Arc<Dentry>> {
        self.upper.get().map(|upper| upper.dentry()).ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay object has no upper real parent")
        })
    }

    /// The per-inode transaction lock; non-directories use it as a plain token.
    pub(self) fn lock(&self) -> MutexGuard<'_, Option<ReaddirCache>> {
        Mutex::lock(&self.lock)
    }

    fn fs_arc(&self) -> Arc<OverlayFs> {
        self.fs
            .upgrade()
            .expect("the owning fs outlives every overlay inode")
    }

    fn self_arc(&self) -> Result<Arc<OverlayInode>> {
        self.this.upgrade().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the overlay inode is no longer owned by an Arc")
        })
    }

    fn append_write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize> {
        let _guard = self.lock();
        let real = self.visible_source().real_inode();
        let offset = real.size();
        real.write_at(offset, reader, status_flags)
    }

    /// The `Once` flips before the cache rekey, so fresh scans may rebuild their own inodes.
    fn replace_facts(&self, new_upper: RealObject) {
        let Some(fs) = self.fs.upgrade() else {
            // Teardown: with no live mount, only the local upper object is published.
            self.upper.call_once(|| new_upper);
            return;
        };
        let new_key = fs.real_object_key(&new_upper);
        let old_key = fs.real_object_key(self.visible_source());
        self.upper.call_once(|| new_upper);
        fs.inodes().publish_rekey(old_key, new_key, &self.this);
        debug_assert!(
            self.this.upgrade().is_some_and(|committer| {
                fs.inodes()
                    .get(new_key)
                    .is_some_and(|probe| Arc::ptr_eq(&probe, &committer))
            }),
            "after replace_facts the inode cache maps the new visible-source key to THIS inode"
        );
    }

    /// Delegates one operation to the visible-source real authority, with its own dentry.
    fn delegate_to_real<T>(
        &self,
        operation_fn: impl FnOnce(&Arc<dyn Inode>, &Dentry) -> Result<T>,
    ) -> Result<T> {
        let visible_source = self.visible_source();
        let real = visible_source.real_inode().clone();
        operation_fn(&real, visible_source.dentry())
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

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.readdir_at_impl(offset, visitor)
    }
}

impl Inode for OverlayInode {
    fn size(&self) -> usize {
        self.visible_source().real_inode().size()
    }

    fn metadata(&self) -> Result<Metadata> {
        let mut metadata = self.visible_source().real_inode().metadata()?;
        metadata.ino = self.object_id.ino;
        metadata.container_dev_id = self.object_id.dev;
        Ok(metadata)
    }

    fn ino(&self) -> u64 {
        self.object_id.ino
    }

    fn type_(&self) -> InodeType {
        self.visible_source().real_inode().type_()
    }

    fn mode(&self) -> Result<InodeMode> {
        self.visible_source().real_inode().mode()
    }

    fn owner(&self) -> Result<Uid> {
        self.visible_source().real_inode().owner()
    }

    fn group(&self) -> Result<Gid> {
        self.visible_source().real_inode().group()
    }

    fn atime(&self) -> Duration {
        self.visible_source().real_inode().atime()
    }

    fn mtime(&self) -> Duration {
        self.visible_source().real_inode().mtime()
    }

    fn ctime(&self) -> Duration {
        self.visible_source().real_inode().ctime()
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let _dir_guard = self.lock();
        if _dir_guard.is_none() {
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

    fn check_permission(&self, perm: Permission) -> Result<()> {
        self.check_permission(AccessType::ReadOnly, perm)
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
        let op = CreateOp::general(type_, mode)?;
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
        let op = CreateOp::mknod(&type_, mode)?;
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
