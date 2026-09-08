// SPDX-License-Identifier: MPL-2.0
#![short_vis_path::add(overlayfs)]

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
//! | [`readdir`] | The merged-directory readdir index and enumeration. |
//! | [`xattr`] | The xattr private-record and passthrough paths. |
//!
//! # Locking
//!
//! `lock` is the per-inode transaction lock; directories carry a
//! [`ReaddirIndex`] in its payload, while non-directories use it as a plain
//! serialization token. [`OverlayInode::append_write`] holds this lock across
//! the underlying `size()` + `write_at` so concurrent appends serialize on the
//! post-write size.
//!
//! The copy-up frame orders its locks strictly: the object's `copyup`
//! winner/waiter mutex, then the publication parent's directory transaction
//! lock, then the `InodeCache` write guard (the innermost leaf). Coordinate
//! extraction precedes every overlay lock: the anchor re-derivation behind
//! dentry-less extraction touches only the VFS dcache (NameAndParent leaf
//! reads) and the `InodeCache` leaf guard. No other lock domains exist: the
//! per-inode transaction lock, the `copyup` mutex, and the `InodeCache` write
//! guard are the only overlay-internal locks.

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

use spin::Once;

pub(super) use self::{
    dir::whiteout::WhiteoutCache,
    identity::{IdentityPolicy, collect_layer_devs},
    inode_cache::InodeCache,
    xattr::{OverlayRecordName, OverlayXattrPrefix, overlay_record_name},
};
use self::{
    identity::ObjectId,
    lookup::{Lookup, NegativeLookup, is_opaque_directory, is_whiteout_inode},
    permission::AccessType,
    readdir::ReaddirIndex,
};
use crate::{
    fs::{
        file::{
            AccessMode, InodeMode, InodeType, PerOpenFileOps, Permission, StatusFlags, SyncMode,
        },
        fs_impls::overlayfs::{
            fs::OverlayFs,
            layer::RealObjectStack,
            real::{RealObject, RealObjectKey},
        },
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{
                Extension, FallocMode, FileOps, Inode, Metadata, MknodType, RenameMode,
                SymbolicLink,
            },
            path::{Dentry, Path},
            xattr::{XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::Vmo,
};

fn mknod_object_type(mknod: &MknodType) -> InodeType {
    match mknod {
        MknodType::NamedPipe => InodeType::NamedPipe,
        MknodType::CharDevice(_) => InodeType::CharDevice,
        MknodType::BlockDevice(_) => InodeType::BlockDevice,
    }
}

pub(super) struct OverlayInode {
    fs: Weak<OverlayFs>,
    lowers: Vec<RealObject>,
    upper: Once<RealObject>,
    object_id: ObjectId,
    lock: Mutex<Option<ReaddirIndex>>,
    /// The winner/waiter token serializing concurrent first copy-ups; it
    /// carries no payload — the sole published-state fact is `upper`.
    copyup: Mutex<()>,
    extension: Extension,
}

impl OverlayInode {
    pub(super) fn new_root(fs: Weak<OverlayFs>) -> Arc<dyn Inode> {
        let fs = match fs.upgrade() {
            Some(fs) => fs,
            None => unreachable!(
                "the root inode is constructed only through a live mount Arc; \
                 the mount reference is always alive at this call site"
            ),
        };
        let layer_stack = &fs.layer_stack();
        let upper = layer_stack
            .upper_layer()
            .ok()
            .map(|layer| RealObject::new(0, layer.root_dentry().clone()));
        let lowers: Vec<_> = layer_stack
            .lower_layers()
            .iter()
            .enumerate()
            .map(|(layer_index, layer)| {
                RealObject::new(layer_index + 1, layer.root_dentry().clone())
            })
            .collect();
        fs.project_inode(&RealObjectStack::new(upper, lowers))
    }

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

    fn contains_real_inode(&self, real_inode: &Arc<dyn Inode>) -> bool {
        Arc::ptr_eq(self.visible_source().real_inode(), real_inode)
            || self
                .lowers
                .iter()
                .any(|lower| Arc::ptr_eq(lower.real_inode(), real_inode))
    }

    fn real_object_stack(&self) -> RealObjectStack {
        RealObjectStack::new(self.upper.get().cloned(), self.lowers.clone())
    }

    fn object_id(&self) -> ObjectId {
        self.object_id
    }

    fn select_real_inode(&self) -> Arc<dyn Inode> {
        self.visible_source().real_inode().clone()
    }

    fn upper_parent_path(&self) -> Result<Path> {
        let fs = self.fs_arc()?;
        let upper = self.upper.get().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay object has no upper real parent")
        })?;
        Ok(fs.real_object_path(upper))
    }

    fn fs_arc(&self) -> Result<Arc<OverlayFs>> {
        let fs = self.fs();
        Arc::downcast::<OverlayFs>(fs).map_err(|_| {
            Error::with_message(
                Errno::EIO,
                "the inode does not belong to an overlay filesystem",
            )
        })
    }

    fn cached_self_arc(&self) -> Result<Arc<OverlayInode>> {
        let fs = self.fs_arc()?;
        fs.inodes().get(self.key(&fs)).ok_or_else(|| {
            Error::with_message(
                Errno::EIO,
                "this overlay inode is not registered under its visible-source key",
            )
        })
    }

    fn append_write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize> {
        let _guard = self.lock.lock();
        let real = self.select_real_inode();
        let offset = real.size();
        real.write_at(offset, reader, status_flags)
    }

    /// The `Once` publication flips before the cache rekey lands. In the
    /// transient window between the two, the upper is published on this
    /// inode but not yet rekeyed in the cache, so fresh lower-only scans
    /// legitimately rebuild their own overlay inodes instead of reusing
    /// this one.
    fn replace_facts(&self, new_upper: RealObject, pin: &Arc<OverlayInode>) {
        let Some(fs) = self.fs.upgrade() else {
            // Teardown arm: with no live mount no lookup can observe this
            // inode, so only the local upper object is published.
            self.upper.call_once(|| new_upper);
            return;
        };
        let new_key = fs.real_object_key(&new_upper);
        let old_key = fs.real_object_key(self.visible_source());
        let old_real_inode = self.visible_source().real_inode().clone();
        self.upper.call_once(|| new_upper);
        fs.inodes()
            .publish_rekey(old_key, new_key, old_real_inode, pin);
        debug_assert!(
            fs.inodes()
                .get(new_key)
                .is_some_and(|probe| Arc::ptr_eq(&probe, pin)),
            "after replace_facts the inode cache maps the new visible-source key to THIS inode"
        );
    }

    /// Delegates one operation to the visible-source real authority, pairing
    /// the real inode with its own dentry for the dentry-centric real-layer
    /// setters.
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
        let _dir_guard = self.lock.lock();
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
        match self.fs.upgrade() {
            Some(fs) => fs,
            None => unreachable!("a live OverlayInode keeps its OverlayFs alive"),
        }
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
        self.create_impl(self_dentry, name, type_, mode)
    }

    fn create_symlink(
        &self,
        self_dentry: &Dentry,
        name: &str,
        target: &str,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        self.create_symlink_impl(self_dentry, name, target, mode)
    }

    fn mknod(
        &self,
        self_dentry: &Dentry,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        self.mknod_impl(self_dentry, name, mode, type_)
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
