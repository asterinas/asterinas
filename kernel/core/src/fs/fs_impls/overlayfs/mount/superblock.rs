// SPDX-License-Identifier: MPL-2.0

//! The overlay filesystem object and its VFS-facing superblock surface.
//!
//! This module owns the `OverlayFs` struct (the published mount/layer/policy
//! state plus the projection state — `bindings`/`inodes`/`identity`), the
//! `FileSystem` trait implementation, and the `MOUNT` lifecycle domain
//! (`MountLifecycle`/`MountPhase`) — `MOUNT` is the mount-lifecycle lock. All fallible mount work happens in
//! `build.rs` (`OverlayFs::new`); the hooks here enter through a pinned
//! `Arc<OverlayFs>` and hold no overlay lock except the short `MOUNT`
//! transition inside [`OverlayFs::begin_shutdown`].

use core::sync::atomic::AtomicU64;

use super::{
    OVERLAY_FS_NAME, claims::UpperWorkdirClaim, layers::OverlayLayerStack, policy::MountPolicy,
};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            dir::whiteout::WhiteoutCache,
            metadata_security::xattr::OverlayXattrPolicy,
            projection::{BindingCache, IdentityPolicy, InodeCache},
        },
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

/// The top-level overlay filesystem object (mirrors Linux `ovl_fs`).
///
/// `OverlayFs` is the only object that publishes mount/layer/policy state to
/// sibling modules. It is created by [`OverlayFs::new`] (in `build.rs`)
/// through the construction sequence; after publication the layer stack,
/// claims, and policy snapshot are immutable. The projection state —
/// `bindings` ([`BindingCache`]), `inodes` ([`InodeCache`]), and `identity`
/// ([`IdentityPolicy`]) — is initialized in the same constructor and consumed
/// by the `projection` module.
///
/// Invariants: `root_inode()` returns the prepared root inode and performs
/// no fallible work; `claims` is `Some` only for writable mounts and is
/// released exactly once on the final `Drop` (guard `Drop`, atomic
/// non-blocking, no mutex); the `MOUNT` lifecycle is used only for lifecycle
/// transitions and is never held across underlying callbacks. The
/// `bindings`/`inodes` caches use sleep-capable `RwMutex` internal data locks
/// (not topology levels) and the `identity` policy is immutable after
/// construction.
///
/// The cross-module shared state for copy-up, metadata security, and namespace
/// mutation — `workdir_temp_serial`, `xattr_policy`, `whiteout_cache` — are
/// also owned here and consumed by their owning modules.
pub(in crate::fs::fs_impls::overlayfs) struct OverlayFs {
    /// The immutable resolved layer stack (upper + lowers).
    pub(super) layer_stack: OverlayLayerStack,
    /// The claimed upper/workdir pair; `Some` only for writable mounts.
    ///
    /// Established single-threaded before publication and released by the
    /// final `Drop` (guard `Drop`, no mutex). The claim additionally pins the
    /// prepared workdir staging workspace inode (`<workdir>/work`) once
    /// `prepare_workdir` completes — a plain `Arc` pin with no lock domain.
    pub(super) claims: Option<UpperWorkdirClaim>,
    /// The immutable published mount policy snapshot.
    pub(super) policy: MountPolicy,
    /// The reported mount source.
    pub(super) mount_source: String,
    /// The prepared root inode.
    ///
    /// The root inode needs the published mount (`fs.layer_stack()` /
    /// `fs.identity()`), but `Weak::upgrade()` is documented-`None` inside
    /// the `Arc::new_cyclic` closure (the strong count stays 0 until the
    /// closure returns). `OverlayFs::new` fills this construction/publication
    /// slot immediately after the `Arc` is published. `root_inode()` only
    /// clones the prepared root; a `None` value for a published mount is a
    /// hard construction invariant failure, never a silent mount-less root.
    pub(super) root_inode: Mutex<Option<Arc<dyn Inode>>>,
    /// The mount lifecycle state; phase only, sleep-capable.
    pub(super) lifecycle: Mutex<MountLifecycle>,
    /// Mount-wide filesystem event subscriber statistics.
    pub(super) fs_event_stats: FsEventSubscriberStats,
    /// The canonical weak mount reference.
    ///
    /// Established by `Arc::new_cyclic` in `OverlayFs::new` (ramfs
    /// `Arc::new_cyclic` + `Weak<RamFs>` precedent) and consumed by
    /// `projection::project_inode` to stamp created `OverlayInode`s with the
    /// mount's live `Weak` — replacing a downcast from the root inode. The weak
    /// never pins the mount.
    pub(in crate::fs::fs_impls::overlayfs) self_weak: Weak<OverlayFs>,
    /// The mount-wide binding cache — the first source for `(parent, name)`
    /// lookup results.
    ///
    /// Entries are immutable `Arc<Binding>` snapshots (a positive pins its
    /// inode, a negative pins its barrier); insert/update happen under the
    /// caller's parent `DIR` (per-parent directory transaction) lock. Not a second layer registry or
    /// identity table.
    pub(in crate::fs::fs_impls::overlayfs) bindings: BindingCache,
    /// The mount-wide inode identity-reuse cache.
    ///
    /// Maps each `RealObjectKey` to a `Weak<OverlayInode>`; weak values so
    /// the cache never forms an `OverlayFs → OverlayInode → OverlayFs` strong
    /// cycle.
    pub(in crate::fs::fs_impls::overlayfs) inodes: InodeCache,
    /// The immutable dev/ino projection policy.
    ///
    /// Built once in `OverlayFs::new` (overlay `st_dev` plus the
    /// construction-local identity tuples); the fallback ino allocator is a
    /// saturating `AtomicU64` inside the policy.
    pub(in crate::fs::fs_impls::overlayfs) identity: IdentityPolicy,
    /// The overlay `AnonDeviceId` RAII guard, retained for the mount lifetime.
    ///
    /// `IdentityPolicy::overlay_dev_id` copies the device id, so the guard
    /// must live on the published `OverlayFs` (the substrate-idiomatic owner —
    /// every Asterinas pseudo-fs and the legacy overlayfs hold `AnonDeviceId`
    /// on the fs struct) or the minor number could be recycled under a live
    /// mount. The `_`-prefixed name mirrors the sibling pseudo-fs precedent
    /// and suppresses the unused-field lint.
    pub(in crate::fs::fs_impls::overlayfs) _anon_device_id: AnonDeviceId,
    /// The saturating workdir temp-name serial.
    ///
    /// Unique-naming context for the copy-up module's
    /// `OverlayFs::generate_workdir_temp_name`: the value is
    /// saturating-fetched and never gates I/O.
    pub(in crate::fs::fs_impls::overlayfs) workdir_temp_serial: AtomicU64,
    /// The immutable xattr classification policy.
    ///
    /// Owned once here; stateless, no lock. Consumed by the
    /// `metadata_security` module.
    pub(in crate::fs::fs_impls::overlayfs) xattr_policy: OverlayXattrPolicy,
    /// The mount-scoped reusable whiteout cache.
    ///
    /// Bounded to one workdir staging slot; whiteout-lock critical sections
    /// never cover BIO/sleep/underlying calls. Consumed by the `dir` module's
    /// short-slot protocol.
    pub(in crate::fs::fs_impls::overlayfs) whiteout_cache: Mutex<WhiteoutCache>,
}

/// The `MOUNT` lifecycle state of an [`OverlayFs`].
///
/// Carries only the phase; the claims are intentionally not mutex-guarded
/// (they are released by guard `Drop` on the final `Drop`, never by a
/// lifecycle transition).
#[derive(Debug)]
pub(super) struct MountLifecycle {
    pub(super) phase: MountPhase,
}

/// The `MOUNT` lifecycle phase of an overlay mount.
///
/// `Ready` is the construction-time phase; [`OverlayFs::begin_shutdown`]
/// performs the only transition, `Ready` → `ShuttingDown`. The final release
/// is the last-`Drop` RAII boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MountPhase {
    /// The mount is live and accepts operations.
    Ready,
    /// The mount is draining; no new operations may start.
    #[expect(dead_code, reason = "the VFS exposes no filesystem shutdown callback")]
    ShuttingDown,
}

impl OverlayFs {
    /// Transitions the `MOUNT` lifecycle from `Ready` to `ShuttingDown`.
    ///
    /// Returns `EBUSY` if the mount is already shutting down. Claim release
    /// happens only on the final `Drop` (after pinned consumers drain), so no
    /// consumer can observe a half-released claim.
    // TODO: Invoke this from the VFS unmount/shutdown callback before detach.
    #[expect(dead_code, reason = "the VFS exposes no filesystem shutdown callback")]
    pub(super) fn begin_shutdown(&self) -> Result<()> {
        let mut lifecycle = self.lifecycle.lock();
        if lifecycle.phase == MountPhase::ShuttingDown {
            return Err(Error::new(Errno::EBUSY));
        }
        lifecycle.phase = MountPhase::ShuttingDown;
        Ok(())
    }

    /// Returns the immutable layer stack.
    ///
    /// Consumed by the `projection` module from `OverlayInode::new_root`.
    pub(in crate::fs::fs_impls::overlayfs) fn layer_stack(&self) -> &OverlayLayerStack {
        &self.layer_stack
    }

    /// Returns the immutable mount policy snapshot.
    ///
    /// Consumed by `OverlayInode::read_only_gate` and
    /// `OverlayFs::store_lower_id`.
    pub(in crate::fs::fs_impls::overlayfs) fn policy(&self) -> &MountPolicy {
        &self.policy
    }

    /// Returns the claimed upper/workdir pair, if this is a writable mount.
    ///
    /// No `projection` caller today.
    pub(in crate::fs::fs_impls::overlayfs) fn claims(&self) -> Option<&UpperWorkdirClaim> {
        self.claims.as_ref()
    }

    /// Returns the real filesystem that superblock hooks forward to: the upper
    /// filesystem for writable mounts, otherwise the topmost lower layer.
    ///
    /// `sync`/`statfs` semantics are forwarded to this filesystem. The
    /// topmost lower is `lowers[0]`, which is guaranteed non-empty by the
    /// checked `OverlayLayerStack::assemble` constructor.
    fn selected_real_fs(&self) -> &Arc<dyn FileSystem> {
        self.layer_stack
            .upper
            .as_ref()
            .map_or(&self.layer_stack.lowers[0].fs, |upper| &upper.fs)
    }
}

impl FileSystem for OverlayFs {
    fn name(&self) -> &'static str {
        OVERLAY_FS_NAME
    }

    fn source(&self) -> Option<&str> {
        Some(self.mount_source.as_str())
    }

    fn sync(&self) -> Result<()> {
        self.selected_real_fs().sync()
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        let root_inode = self.root_inode.lock();
        match root_inode.as_ref() {
            Some(root) => root.clone(),
            // `OverlayFs::new` fills the slot right after publishing the
            // `Arc`, so a published mount always carries its root; a missing
            // slot is a construction-order violation, never a runtime
            // condition (hard invariant, no `.unwrap()`/`.expect()`).
            None => unreachable!(
                "OverlayFs::new materializes the root inode before publication; \
                 a published overlay mount always has its root slot set"
            ),
        }
    }

    fn sb(&self) -> SuperBlock {
        let mut super_block = self.selected_real_fs().sb();
        if let Some(uuid) = self.policy().uuid() {
            super_block.fsid = uuid.value();
        }
        super_block
    }

    fn flags(&self) -> FsFlags {
        if self.policy().is_effective_read_only() {
            FsFlags::RDONLY
        } else {
            FsFlags::empty()
        }
    }

    fn set_fs_flags(&self, flags: FsFlags, _data: Option<&str>, _ctx: &Context) -> Result<()> {
        // The effective read-only state is fixed at mount time and only
        // reported by `flags()`; full remount semantics are not implemented,
        // so any delta is rejected instead of being silently accepted.
        let current_flags = self.flags();
        if current_flags.contains(FsFlags::RDONLY) && !flags.contains(FsFlags::RDONLY) {
            return Err(Error::new(Errno::EROFS));
        }
        if flags != current_flags {
            return Err(Error::with_message(
                Errno::EINVAL,
                "unsupported overlayfs remount delta",
            ));
        }
        Ok(())
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_stats
    }
}
