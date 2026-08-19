// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The Overlay inode and its canonical VFS trait surface.
//!
//! An [`OverlayInode`] is the published logical inode: one overlay object
//! shared by every name bound to it. [`ObjectFacts`] is the
//! per-object real-object facts — its per-name kind, the upper real object
//! (the visible-metadata source for merged directories), and the
//! topmost-first lower stack — replaced only by the copy-up transition.
//! The module also owns the root-inode constructor
//! ([`OverlayInode::new_root`]) and the sole `Inode` and `FileOps`
//! implementations of the overlay.
//!
//! # Locking
//!
//! `dir_transaction_lock` serializes directory mutations (present only on
//! directories). `facts` guards the per-object facts and is normally held
//! only briefly; the one non-obvious hold is [`OverlayInode::append_write`],
//! which keeps the `facts` guard across the underlying `size()` + `write_at`
//! so concurrent appends serialize on the post-write size.
//!
//! # Structure
//!
//! | Item | Owns |
//! |---|---|
//! | [`OverlayInode`] | The published logical inode and its `Inode`/`FileOps` surfaces. |
//! | [`ObjectFacts`] | The immutable real-object facts. |
//!
//! # References
//!
//! - Overlayfs (Linux overlay filesystem):
//!   <https://www.kernel.org/doc/html/latest/filesystems/overlayfs.html>

use core::time::Duration;

use crate::{
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, Permission, StatusFlags},
        fs_impls::overlayfs::{
            AccessType,
            copyup::coordination::CopyUpTransition,
            projection::{ObjectId, PositiveKind, RealObject, RealObjectKey},
            readdir_index::ReaddirIndex,
            superblock::OverlayFs,
        },
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{
                Extension, FallocMode, FileOps, Inode, Metadata, MknodType, RenameMode,
                RevalidationPolicy, SymbolicLink,
            },
            path::Path,
            xattr::{XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::Vmo,
};

/// The logical Overlay inode exposed to the VFS: one logical overlay object
/// shared by every name bound to it, with the real-object facts living once
/// in [`ObjectFacts`].
pub(super) struct OverlayInode {
    /// The owning mount.
    pub(super) fs: Weak<OverlayFs>,
    /// The inode-cache key of the visible-metadata source.
    pub(super) key: Mutex<RealObjectKey>,
    /// The per-object real-object facts, replaced only by the copy-up
    /// transition.
    pub(super) facts: Mutex<ObjectFacts>,
    /// The per-directory transaction lock; `Some` iff this object is a
    /// directory.
    pub(super) dir_transaction_lock: Option<Mutex<()>>,
    /// The precomputed projected `st_dev`/`st_ino`.
    pub(super) object_id: ObjectId,
    /// The VFS inode extension groups (fs event publisher / fs lock context).
    pub(super) extension: Extension,
    /// The per-directory merged-readdir index; `Some` iff this object is a
    /// directory.
    pub(super) readdir_index: Option<Mutex<ReaddirIndex>>,
    /// The copy-up transition coordinate; `None` until copy-up records the
    /// first positive-binding publication.
    pub(super) copyup_transition: Mutex<Option<CopyUpTransition>>,
}

/// The immutable real-object facts of one logical overlay object.
///
/// Invariant: `upper.is_some() || !lowers.is_empty()`,
/// enforced at the construction paths
/// (the in-tree `projection` builders and [`ObjectFacts::try_new`]).
///
/// Literal construction is allowed only on paths already satisfying the invariant;
/// `try_new` remains the checked entry for fallible construction.
#[derive(Clone, Debug)]
pub(super) struct ObjectFacts {
    /// The per-name view classification of this object.
    pub(super) kind: PositiveKind,
    /// The upper real object; the visible-metadata source for merged
    /// directories.
    pub(super) upper: Option<RealObject>,
    /// The lower stack, topmost first; non-empty for lower-only/merged
    /// objects.
    pub(super) lowers: Vec<RealObject>,
}

impl ObjectFacts {
    /// Constructs an [`ObjectFacts`], returning `None` when both
    /// `upper` and `lowers` are empty.
    pub(super) fn try_new(
        kind: PositiveKind,
        upper: Option<RealObject>,
        lowers: Vec<RealObject>,
    ) -> Option<Self> {
        if upper.is_some() || !lowers.is_empty() {
            Some(Self {
                kind,
                upper,
                lowers,
            })
        } else {
            None
        }
    }

    /// Returns whether `self` and `other` share the same visible identity
    /// by cache-alias pointer identity.
    ///
    /// Kinds and upper identities must match;
    /// `Single` objects compare only the visible source
    /// (post-copy-up inodes retain bookkeeping lowers),
    /// and `Merged` objects compare the full lower composition strictly.
    /// For durable value-identity revalidation, use
    /// [`ObjectFacts::same_layer_composition`].
    pub(super) fn same_visible_identity(&self, other: &Self) -> bool {
        if self.kind != other.kind {
            return false;
        }
        let same_upper = match (self.upper.as_ref(), other.upper.as_ref()) {
            (Some(left), Some(right)) => Arc::ptr_eq(left.real_inode(), right.real_inode()),
            (None, None) => true,
            _ => false,
        };
        if !same_upper {
            return false;
        }
        match self.kind {
            PositiveKind::Single => Arc::ptr_eq(
                self.visible_source().real_inode(),
                other.visible_source().real_inode(),
            ),
            PositiveKind::Merged => {
                self.lowers.len() == other.lowers.len()
                    && self
                        .lowers
                        .iter()
                        .zip(other.lowers.iter())
                        .all(|(left, right)| Arc::ptr_eq(left.real_inode(), right.real_inode()))
            }
        }
    }

    /// Returns whether `self` and `other` describe the same physical layer
    /// composition by durable value identity (`fsid` + real inode number),
    /// not by cached inode identity.
    ///
    /// This is the lock-free revalidation comparator: snapshots taken at
    /// different moments may hold different `Arc`s for the same physical
    /// object, so pointer identity is too strict. `kind`, the upper layer,
    /// and every lower layer must match by `fsid`/`ino`.
    pub(super) fn same_layer_composition(&self, other: &Self) -> bool {
        let same_upper = match (self.upper.as_ref(), other.upper.as_ref()) {
            (Some(left), Some(right)) => {
                left.fsid() == right.fsid() && left.real_inode().ino() == right.real_inode().ino()
            }
            (None, None) => true,
            _ => false,
        };
        self.kind == other.kind
            && same_upper
            && self.lowers.len() == other.lowers.len()
            && self
                .lowers
                .iter()
                .zip(other.lowers.iter())
                .all(|(left, right)| {
                    left.fsid() == right.fsid()
                        && left.real_inode().ino() == right.real_inode().ino()
                })
    }

    /// Returns whether `real_inode` is the same logical object as this
    /// object's visible source or any of its retained lowers.
    pub(super) fn contains_real_inode(&self, real_inode: &Arc<dyn Inode>) -> bool {
        Arc::ptr_eq(self.visible_source().real_inode(), real_inode)
            || self
                .lowers
                .iter()
                .any(|lower| Arc::ptr_eq(lower.real_inode(), real_inode))
    }

    /// Returns the visible-metadata source: the upper real object when present,
    /// else the topmost lower (`lowers[0]`).
    ///
    /// Precondition: `upper.is_some() || !lowers.is_empty()`.
    pub(super) fn visible_source(&self) -> &RealObject {
        match &self.upper {
            Some(upper) => upper,
            None => &self.lowers[0],
        }
    }

    /// Returns the current real authority for one delegated call.
    pub(super) fn select_real_inode(&self) -> Arc<dyn Inode> {
        self.visible_source().real_inode().clone()
    }
}

impl OverlayInode {
    /// Constructs the overlay mount root inode on demand.
    ///
    /// The root facts merge the upper root with all lower roots; the root is
    /// always a directory.
    pub(super) fn new_root(fs: Weak<OverlayFs>) -> Arc<dyn Inode> {
        let fs = match fs.upgrade() {
            Some(fs) => fs,
            None => unreachable!(
                "the root inode is constructed only through a live mount Arc; \
                 the mount reference is always alive at this call site"
            ),
        };
        let layer_stack = &fs.layer_stack;
        let upper = layer_stack.upper_layer().ok().map(|layer| {
            RealObject::from_layer_path(
                0,
                layer.root_path.clone(),
                layer.fsid,
                layer.container_dev_id,
            )
        });
        let lowers: Vec<_> = layer_stack
            .lower_layers()
            .iter()
            .enumerate()
            .map(|(layer_index, layer)| {
                RealObject::from_layer_path(
                    layer_index + 1,
                    layer.root_path.clone(),
                    layer.fsid,
                    layer.container_dev_id,
                )
            })
            .collect();
        // Merged-root classification: a writable root merges the upper with
        // the lowers; a read-only root merges its lower stack when more than
        // one lower directory participates.
        let kind = if upper.is_some() || lowers.len() > 1 {
            PositiveKind::Merged
        } else {
            PositiveKind::Single
        };
        let facts = ObjectFacts {
            kind,
            upper,
            lowers,
        };
        // The layer stack always carries at least one lower layer, so
        // `visible_source()` never indexes an empty `lowers`.
        let visible = facts.visible_source();
        let key = RealObjectKey::from_facts(&facts);
        let object_id = fs.identity.project_object_id(visible, true);
        let inode = Arc::new(OverlayInode {
            fs: Arc::downgrade(&fs),
            key: Mutex::new(key),
            facts: Mutex::new(facts),
            dir_transaction_lock: Some(Mutex::new(())),
            object_id,
            extension: Extension::new(),
            readdir_index: Some(Mutex::new(ReaddirIndex::new())),
            copyup_transition: Mutex::new(None),
        });
        // Register the root inode in the inode cache so every live inode
        // resolves by its visible-source key; the inlined parent probe needs
        // no root special case and `project_inode` never mints a duplicate.
        fs.inodes.get_or_create(key, |_| true, || inode.clone());
        inode
    }

    pub(super) fn key(&self) -> RealObjectKey {
        *self.key.lock()
    }

    /// Returns the precomputed projected `st_dev`/`st_ino`.
    ///
    /// Copy-up re-projection keeps the lower-id-derived identity, so the
    /// value is stable across copy-up (authority-continuity invariant).
    pub(super) fn object_id(&self) -> ObjectId {
        self.object_id
    }

    pub(super) fn facts_snapshot(&self) -> ObjectFacts {
        self.facts.lock().clone()
    }

    pub(super) fn select_real_inode(&self) -> Arc<dyn Inode> {
        self.facts_snapshot().select_real_inode()
    }

    /// Returns the dentry-anchored path of the promoted upper real parent
    /// directory.
    ///
    /// After promotion the facts guarantee an upper object that is always
    /// dentry-anchored, so the checked `real_path()` accessor succeeds;
    /// `EROFS`/`EIO` propagate when that guarantee does not hold.
    pub(super) fn upper_parent_path(&self) -> Result<Path> {
        let facts = self.facts_snapshot();
        let upper = facts.upper.ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay object has no upper real parent")
        })?;
        upper.real_path()
    }

    /// Returns `Err` when the inode does not belong to an overlay filesystem.
    pub(super) fn fs_arc(&self) -> Result<Arc<OverlayFs>> {
        let fs = self.fs();
        Arc::downcast::<OverlayFs>(fs).map_err(|_| {
            Error::with_message(
                Errno::EIO,
                "the inode does not belong to an overlay filesystem",
            )
        })
    }

    /// Locks the per-object copy-up coordination state.
    pub(super) fn lock_copyup_transition(&self) -> MutexGuard<'_, Option<CopyUpTransition>> {
        self.copyup_transition.lock()
    }

    /// Attempts to lock the per-object copy-up coordination state without
    /// blocking; `None` when another coordinator holds the lock.
    pub(super) fn try_lock_copyup_transition(
        &self,
    ) -> Option<MutexGuard<'_, Option<CopyUpTransition>>> {
        self.copyup_transition.try_lock()
    }

    /// Serializes an `O_APPEND` write as one atomic size-read + write.
    ///
    /// The `facts` lock is held across both steps because the underlying fs
    /// does not process `O_APPEND` itself. This is the one exception to
    /// holding `facts` only briefly, and it serializes concurrent appends
    /// on the post-write size.
    pub(super) fn append_write(
        &self,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let guard = self.facts.lock();
        let real = guard.select_real_inode();
        let offset = real.size();
        real.write_at(offset, reader, status_flags)
    }

    /// Replaces the real-object facts of this inode — the copy-up transition.
    ///
    /// The transition is fallible and self-consistent: the inode-cache
    /// registration is aliased under the new visible-source key while the
    /// old-key mapping is retained, then the facts and published `key` are
    /// swapped. The alias runs first, so a displacement fails rather than
    /// silently orphaning the inode.
    pub(super) fn replace_facts(
        self: &Arc<Self>,
        facts: ObjectFacts,
        new_visible_source: &RealObject,
    ) -> Result<()> {
        let new_key = RealObjectKey::from_facts(&facts);
        // Capture the pre-transition visible-source key AND its real inode
        // under one brief `facts` lock: the old real inode becomes the
        // keep-alive pin of the retained old-key alias (`rekey_keep_old_alias`), so it
        // cannot be recycled while the alias exists.
        let (old_key, old_real_inode) = {
            let old_facts = self.facts.lock();
            (
                RealObjectKey::from_facts(&old_facts),
                old_facts.visible_source().real_inode().clone(),
            )
        };
        // A live inode cannot outlive its mount; the teardown arm swaps the
        // facts locally and skips the cache alias (no live lookup can
        // observe this inode then).
        let Some(fs) = self.fs.upgrade() else {
            *self.facts.lock() = facts;
            *self.key.lock() = new_key;
            return Ok(());
        };
        // The fallible alias runs first; only after it succeeds is the
        // inode's own state committed.
        fs.inodes
            .rekey_keep_old_alias(old_key, new_key, old_real_inode, new_visible_source)?;
        *self.facts.lock() = facts;
        *self.key.lock() = new_key;
        debug_assert!(
            fs.inodes
                .get(new_key)
                .is_some_and(|probe| Arc::ptr_eq(&probe, self)),
            "after replace_facts the inode cache maps the new visible-source key to THIS inode"
        );
        if self.dir_transaction_lock.is_some() {
            fs.bindings.invalidate_parent(&old_key);
        }
        Ok(())
    }
}

impl OverlayInode {
    // A lower-backed read passes `O_NOATIME` so a read never updates the lower
    // atime; the `O_NOATIME` decision and the real-inode selection share the
    // same `facts` snapshot below, so no double-snapshot window remains.
    pub(super) fn read_at_impl(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let facts = self.facts_snapshot();
        let is_lower_backed = facts.upper.is_none();
        let real = facts.select_real_inode();
        let status_flags = if is_lower_backed {
            status_flags | StatusFlags::O_NOATIME
        } else {
            status_flags
        };
        real.read_at(offset, writer, status_flags)
    }

    // The `O_APPEND` branch serializes `offset := real size` + `write_at`
    // under the `facts` guard (`append_write`) — a bare two-step
    // size-read-then-write would be a TOCTOU where concurrent appends could
    // read the same size and lose an update. Write-capable fds are upper by
    // construction, so delegation never bypasses the trigger.
    pub(super) fn write_at_impl(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if status_flags.contains(StatusFlags::O_APPEND) {
            return self.append_write(reader, status_flags);
        }
        let real = self.select_real_inode();
        real.write_at(offset, reader, status_flags)
    }

    // The VFS handle uses this inode's own `FileOps`, so the successful path
    // returns `None`; failures surface as `Some(Err)`.
    pub(super) fn open_impl(
        &self,
        access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        if self.type_().is_directory() {
            return None;
        }
        if !access_mode.is_writable() {
            return None;
        }
        let fs = match self.fs_arc() {
            Ok(fs) => fs,
            Err(err) => return Some(Err(err)),
        };
        if fs.policy.is_effective_read_only() {
            return Some(Err(Error::with_message(
                Errno::EROFS,
                "the overlay mount is read-only",
            )));
        }
        match self.ensure_upper_authority() {
            Ok(()) => None,
            Err(err) => Some(Err(err)),
        }
    }

    pub(super) fn seek_end_impl(&self) -> Option<usize> {
        self.select_real_inode().seek_end()
    }

    // The path-based `truncate()` syscall performs no VFS-level `MAY_WRITE`
    // check of its own, so this entry runs the uniform mutating admission
    // before any side effect, including the copy-up promotion.
    pub(super) fn resize_impl(&self, new_size: usize) -> Result<()> {
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.ensure_upper_authority()?;
        self.select_real_inode().resize(new_size)
    }

    // Admission runs uniformly via `check_permission` at this entry:
    // `fallocate` shares `resize`'s side-effect class, so the admission also
    // runs here rather than relying on the fd path alone.
    pub(super) fn fallocate_impl(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.ensure_upper_authority()?;
        self.select_real_inode().fallocate(mode, offset, len)
    }

    pub(super) fn sync_all_impl(&self) -> Result<()> {
        self.select_real_inode().sync_all()
    }

    pub(super) fn sync_data_impl(&self) -> Result<()> {
        self.select_real_inode().sync_data()
    }

    pub(super) fn read_link_impl(&self) -> Result<SymbolicLink> {
        self.select_real_inode().read_link()
    }

    pub(super) fn page_cache_impl(&self) -> Option<Arc<Vmo>> {
        self.select_real_inode().page_cache()
    }
}

impl OverlayInode {
    fn size_impl(&self) -> usize {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().size()
    }

    fn metadata_impl(&self) -> Result<Metadata> {
        let facts = self.facts_snapshot();
        let mut metadata = facts.visible_source().real_inode().metadata()?;
        metadata.ino = self.object_id.ino;
        metadata.container_dev_id = self.object_id.dev;
        Ok(metadata)
    }

    fn ino_impl(&self) -> u64 {
        self.object_id.ino
    }

    fn type_impl(&self) -> InodeType {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().type_()
    }

    fn mode_impl(&self) -> Result<InodeMode> {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().mode()
    }

    fn owner_impl(&self) -> Result<Uid> {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().owner()
    }

    fn group_impl(&self) -> Result<Gid> {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().group()
    }

    fn atime_impl(&self) -> Duration {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().atime()
    }

    fn mtime_impl(&self) -> Duration {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().mtime()
    }

    fn ctime_impl(&self) -> Duration {
        let facts = self.facts_snapshot();
        facts.visible_source().real_inode().ctime()
    }

    fn lookup_impl(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let dir = self.dir_transaction_lock.as_ref().ok_or_else(|| {
            Error::with_message(
                Errno::ENOTDIR,
                "lookup is supported on overlay directories only",
            )
        })?;
        let _dir_guard = dir.lock();
        let facts = self.facts_snapshot();
        let fs = self.fs.upgrade().ok_or_else(|| {
            Error::with_message(Errno::EIO, "the overlay mount is no longer alive")
        })?;
        let binding = fs.lookup_binding(&facts, name)?.binding;
        match binding.inode() {
            Some(inode) => Ok(inode),
            None => Err(Error::new(Errno::ENOENT)),
        }
    }

    fn fs_impl(&self) -> Arc<dyn FileSystem> {
        match self.fs.upgrade() {
            Some(fs) => fs,
            None => unreachable!("a live OverlayInode keeps its OverlayFs alive"),
        }
    }

    /// Returns the revalidation policy for this inode.
    ///
    /// Directories use `REVALIDATE_ABSENT`: an absent name may have appeared
    /// behind the overlay since the last lookup (a lower-layer change or a
    /// concurrent mutation), so a cached negative dentry must be re-checked.
    /// Non-directories return the empty policy: their existence is pinned by
    /// the binding, so no absent-name revalidation applies.
    fn revalidation_policy_impl(&self) -> RevalidationPolicy {
        match self.type_() {
            InodeType::Dir => RevalidationPolicy::REVALIDATE_ABSENT,
            _ => RevalidationPolicy::empty(),
        }
    }

    fn revalidate_absent_impl(&self, _name: &str) -> bool {
        // A negative dentry hit is always re-looked-up.
        false
    }

    fn extension_impl(&self) -> &Extension {
        &self.extension
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
        self.size_impl()
    }

    fn metadata(&self) -> Result<Metadata> {
        self.metadata_impl()
    }

    fn ino(&self) -> u64 {
        self.ino_impl()
    }

    fn type_(&self) -> InodeType {
        self.type_impl()
    }

    fn mode(&self) -> Result<InodeMode> {
        self.mode_impl()
    }

    fn owner(&self) -> Result<Uid> {
        self.owner_impl()
    }

    fn group(&self) -> Result<Gid> {
        self.group_impl()
    }

    fn atime(&self) -> Duration {
        self.atime_impl()
    }

    fn mtime(&self) -> Duration {
        self.mtime_impl()
    }

    fn ctime(&self) -> Duration {
        self.ctime_impl()
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        self.lookup_impl(name)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs_impl()
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        self.revalidation_policy_impl()
    }

    fn revalidate_absent(&self, name: &str) -> bool {
        self.revalidate_absent_impl(name)
    }

    fn extension(&self) -> &Extension {
        self.extension_impl()
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        self.open_impl(access_mode, status_flags)
    }

    fn seek_end(&self) -> Option<usize> {
        self.seek_end_impl()
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.resize_impl(new_size)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.fallocate_impl(mode, offset, len)
    }

    fn sync_all(&self) -> Result<()> {
        self.sync_all_impl()
    }

    fn sync_data(&self) -> Result<()> {
        self.sync_data_impl()
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link_impl()
    }

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        self.page_cache_impl()
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.set_mode_impl(mode)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.set_owner_impl(uid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.set_group_impl(gid)
    }

    fn set_atime(&self, time: Duration) {
        self.set_atime_impl(time)
    }

    fn set_mtime(&self, time: Duration) {
        self.set_mtime_impl(time)
    }

    fn set_ctime(&self, time: Duration) {
        self.set_ctime_impl(time)
    }

    fn check_permission(&self, perm: Permission) -> Result<()> {
        self.check_permission(AccessType::ReadOnly, perm)
    }

    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        self.get_xattr_impl(name, value_writer)
    }

    fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.set_xattr_impl(name, value_reader, flags)
    }

    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        self.list_xattr_impl(namespace, list_writer)
    }

    fn remove_xattr(&self, name: XattrName) -> Result<()> {
        self.remove_xattr_impl(name)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        self.create_impl(name, type_, mode)
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        self.mknod_impl(name, mode, type_)
    }

    fn write_link(&self, target: &str) -> Result<()> {
        self.write_link_impl(target)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        self.link_impl(old, name)
    }

    fn unlink(&self, name: &str) -> Result<()> {
        self.unlink_impl(name)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        self.rmdir_impl(name)
    }

    fn rename(
        &self,
        old_name: &str,
        target: &Arc<dyn Inode>,
        new_name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        self.rename_impl(old_name, target, new_name, mode)
    }
}
