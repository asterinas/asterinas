// SPDX-License-Identifier: MPL-2.0

//! The module root of the copy-up authority and file-views subsystem.
//!
//! This module declares the four `copyup/*` submodules and hosts the thin
//! inode-level delegation entries: the `OverlayFs`-extension impl block (the
//! `workdir_temp_serial` unique-naming accessor), the `OverlayInode`
//! delegation helpers (`select_real_inode`, `fs_arc`,
//! `record_copyup_transition`), and the VFS helper bodies called by the
//! canonical `FileOps`/`Inode` trait impls in `projection/inode.rs`:
//! `read_at_impl`/`write_at_impl`, `open_impl`, `seek_end_impl`,
//! `resize_impl`, `fallocate_impl`, `sync_all_impl`/`sync_data_impl`, and
//! `read_link_impl`/`page_cache_impl`. The real control flow lives in the
//! sibling files: `trigger.rs` (winner/waiter protocol + top-down ancestor
//! walk), `promote.rs` (object-kind promotion body and publication),
//! `workdir.rs` (temp lifecycle).
//!
//! Lock contract: the delegation entries hold no Overlay lock beyond the
//! brief `INODE` facts snapshot inside `select_real_inode`
//! (snapshot-and-release, never held across an underlying call);
//! `record_copyup_transition` takes a brief non-blocking `CUL` `try_lock`;
//! the EROFS gate precedes every promotion side effect. One O_APPEND
//! exception: `write_at_impl` routes the append path to
//! [`OverlayInode::append_write`] (`projection/inode.rs`), which holds the
//! `INODE` facts guard across the underlying real `size()` + `write_at` so
//! concurrent appends serialize on the post-write size. That hold never
//! re-enters an Overlay lock (the real is parsed from the held snapshot, not
//! re-resolved), the underlying fs lock is a leaf that never re-enters
//! Overlay, and the hold is bounded by one write call. No per-open real-inode
//! view object exists: every call re-resolves the current authority per
//! operation (Linux `ovl_real_file_path` follow-copy-up, file.c:128-171).

use core::sync::atomic::Ordering;

use self::coordination::{CopyUpPhase, CopyUpTransition};
use crate::{
    fs::{
        file::{AccessMode, PerOpenFileOps, Permission, StatusFlags},
        fs_impls::overlayfs::{AccessType, mount::OverlayFs, projection::OverlayInode},
        vfs::inode::{FallocMode, Inode, SymbolicLink},
    },
    prelude::*,
    vm::page_cache::Vmo,
};

pub(super) mod coordination;

pub(in crate::fs::fs_impls::overlayfs) mod promote;
mod trigger;
mod workdir;

pub(in crate::fs::fs_impls::overlayfs) use workdir::WorkdirTempRequest;

impl OverlayFs {
    /// Returns the next saturating workdir temp serial.
    ///
    /// The per-mount serial is the unique-naming context of the workdir temp
    /// lifecycle; the consuming `generate_workdir_temp_name`
    /// (`copyup/workdir.rs`) composites it as
    /// `#{target_name}#{parent_ino}#{serial}`. The fetch is saturating —
    /// `AtomicU64::try_update` commits `saturating_add(1)` and retries on
    /// contention, so the counter converges to and stays at `u64::MAX` (the
    /// same pattern as `IdentityPolicy::allocate_fallback_ino`) — and never
    /// gates I/O. Uniqueness is by construction (target name + upper-parent
    /// real ino + per-mount serial); no lock is held (workdir temp naming is
    /// uniqueness-based, not lock-based).
    pub(super) fn workdir_temp_serial(&self) -> u64 {
        match self
            .workdir_temp_serial
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(1))
            }) {
            // The closure never returns `None`, so `try_update` always
            // succeeds; this arm is defensive and unreachable.
            Ok(previous) => previous.saturating_add(1),
            Err(_) => u64::MAX,
        }
    }
}

impl OverlayInode {
    /// Resolves the current authority's real inode for one delegated call.
    ///
    /// A brief `INODE` facts snapshot selects `facts.upper` when present, else
    /// the topmost lower (`lowers[0]`); the guard is released before any
    /// underlying call and the returned strong pin keeps the resolved real
    /// inode alive for the delegation. Every operation re-resolves this way,
    /// so an fd opened while lower-backed observes the upper real inode on
    /// its next operation after a copy-up (Linux `ovl_real_file_path`,
    /// file.c:128-171). The `lowers[0]` index is safe by the facts invariant
    /// `upper.is_some() || !lowers.is_empty()`.
    pub(super) fn select_real_inode(&self) -> Arc<dyn Inode> {
        let facts = self.facts_snapshot();
        match facts.upper() {
            Some(upper) => upper.real_inode().clone(),
            None => facts.lowers()[0].real_inode().clone(),
        }
    }

    /// Upgrades the owning mount's `Weak` reference into an `Arc`.
    ///
    /// The upgrade routes through the public `Inode::fs()` surface — the only
    /// mount route a sibling module can name, since the `OverlayInode::fs`
    /// field stays `pub(super)` inside `projection` — and downcasts the
    /// `Arc<dyn FileSystem>` to `Arc<OverlayFs>`.
    /// The downcast cannot fail for an `OverlayInode` (its `fs` field is a
    /// `Weak<OverlayFs>`); the failure arm is defensive. The post-teardown
    /// failure arm is the platform-lifetime question carried verbatim by
    /// `Inode::fs()` (`unreachable!`); no `.unwrap()`/`.expect()` is
    /// introduced.
    pub(super) fn fs_arc(&self) -> Result<Arc<OverlayFs>> {
        let fs = self.fs();
        Arc::downcast::<OverlayFs>(fs).map_err(|_| {
            Error::with_message(
                Errno::EIO,
                "the inode does not belong to an overlay filesystem",
            )
        })
    }

    /// Records the copy-up transition coordinate at the first positive
    /// binding publication (invoked from `OverlayFs::lookup_binding` before
    /// `publish_binding`).
    ///
    /// The coordinate (`publication_parent` + `name`) is set once — the first
    /// positive binding wins — and is immutable thereafter. The guard is a
    /// non-blocking `try_lock` that skips when contended: contention implies a
    /// transition is already running, hence the coordinate is already set
    /// (waiters hold nothing while blocked). The initial phase is
    /// [`CopyUpPhase::Idle`].
    pub(super) fn record_copyup_transition(
        &self,
        publication_parent: Arc<OverlayInode>,
        name: &str,
    ) {
        let Some(mut guard) = self.copyup_transition.try_lock() else {
            return;
        };
        if guard.is_some() {
            return;
        }
        *guard = Some(CopyUpTransition {
            publication_parent,
            name: String::from(name),
            phase: CopyUpPhase::Idle,
        });
    }
}

impl OverlayInode {
    // Read delegation: per-call authority re-resolution; a lower-backed read
    // passes `O_NOATIME` so a read never updates the lower atime. The two
    // brief facts snapshots (here and inside `select_real_inode`) may observe
    // an authority advance between them, which is benign; no Overlay lock is
    // held across any underlying call.
    pub(in crate::fs::fs_impls::overlayfs) fn read_at_impl(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let facts = self.facts_snapshot();
        let is_lower_backed = facts.upper().is_none();
        let real = self.select_real_inode();
        let status_flags = if is_lower_backed {
            status_flags | StatusFlags::O_NOATIME
        } else {
            status_flags
        };
        real.read_at(offset, writer, status_flags)
    }

    // Write delegation: per-call authority re-resolution; the `O_APPEND`
    // branch serializes `offset := real size` + `write_at` under the `INODE`
    // facts guard (`append_write`) — a bare two-step size-read-then-write
    // would be a TOCTOU where two concurrent appends could read the same
    // size and lose an update. The non-`O_APPEND` branch re-resolves the
    // authority and writes at the passed offset. Write-capable fds are upper
    // by construction, so delegation never bypasses the trigger.
    pub(in crate::fs::fs_impls::overlayfs) fn write_at_impl(
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
}

impl OverlayInode {
    // Directory opens are served by the merged readdir path and read-only
    // opens take no side effect; only writable opens reach the EROFS gate and
    // the write-intent promotion trigger. The VFS handle uses this inode's
    // own `FileOps`, so the successful path returns `None`; failures surface
    // as `Some(Err)`.
    pub(in crate::fs::fs_impls::overlayfs) fn open_impl(
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
        if fs.policy().is_effective_read_only() {
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

    // The end position of the current authority's real inode.
    pub(in crate::fs::fs_impls::overlayfs) fn seek_end_impl(&self) -> Option<usize> {
        self.select_real_inode().seek_end()
    }

    // Truncate leg: EROFS, then the uniform mutating admission (the
    // path-based `truncate()` syscall performs no VFS-level `MAY_WRITE` check
    // of its own, so this entry must run the two-stage admission BEFORE any
    // side effect, including the copy-up promotion), then the promotion
    // trigger and delegation to the (upper) current authority.
    pub(in crate::fs::fs_impls::overlayfs) fn resize_impl(&self, new_size: usize) -> Result<()> {
        if self.fs_arc()?.policy().is_effective_read_only() {
            return_errno_with_message!(Errno::EROFS, "the overlay mount is read-only");
        }
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.ensure_upper_authority()?;
        self.select_real_inode().resize(new_size)
    }

    // EROFS, then the uniform mutating admission (gate independence:
    // `fallocate` shares `resize`'s side-effect class, so the admission runs
    // here too rather than relying on the fd path alone), then the promotion
    // trigger and delegation to the (upper) current authority.
    pub(in crate::fs::fs_impls::overlayfs) fn fallocate_impl(
        &self,
        mode: FallocMode,
        offset: usize,
        len: usize,
    ) -> Result<()> {
        if self.fs_arc()?.policy().is_effective_read_only() {
            return_errno_with_message!(Errno::EROFS, "the overlay mount is read-only");
        }
        self.check_permission(AccessType::Mutating, Permission::MAY_WRITE)?;
        self.ensure_upper_authority()?;
        self.select_real_inode().fallocate(mode, offset, len)
    }

    // Pure delegation to the current authority; no promotion (durability
    // policy is auto).
    pub(in crate::fs::fs_impls::overlayfs) fn sync_all_impl(&self) -> Result<()> {
        self.select_real_inode().sync_all()
    }

    // Same delegation as `sync_all`; durability policy is auto.
    pub(in crate::fs::fs_impls::overlayfs) fn sync_data_impl(&self) -> Result<()> {
        self.select_real_inode().sync_data()
    }

    // Pure delegation to the current authority; no promotion.
    pub(in crate::fs::fs_impls::overlayfs) fn read_link_impl(&self) -> Result<SymbolicLink> {
        self.select_real_inode().read_link()
    }

    // Pure forwarder to the current authority's real page cache (upper after
    // promotion; the lower source for lower-backed read views). Never
    // promotes: the parameterless entry point carries no write intent.
    pub(in crate::fs::fs_impls::overlayfs) fn page_cache_impl(&self) -> Option<Arc<Vmo>> {
        self.select_real_inode().page_cache()
    }
}
