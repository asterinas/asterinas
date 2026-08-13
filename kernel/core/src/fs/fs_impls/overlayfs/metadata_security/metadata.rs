// SPDX-License-Identifier: MPL-2.0

//! The metadata-mutation entries.
//!
//! This module hosts the uniform mutating entry shape for the six metadata
//! setters: `set_mode`/`set_owner`/`set_group` (chmod/chown) and
//! `set_atime`/`set_mtime`/`set_ctime` (utimes). Every entry admits through
//! the two-stage permission pipeline (`self.check_permission(AccessType::
//! Mutating, ...)`, defined in the sibling `permission.rs`) and then forwards
//! to the current real authority under the mount's creator-credential scope
//! through the single private delegation helper `delegate_to_real` (defined
//! in `mod.rs` so the three sibling files share it). The three
//! ownership-sensitive setters admit with `Permission::empty()` (the
//! ownership/capability gate is authoritative; Linux requires no write access
//! for chmod/chown). The three time setters admit with the Linux-faithful
//! utimensat disjunction: `Permission::MAY_WRITE` OR ownership OR
//! `CAP_FOWNER` — the `MAY_WRITE` admission (EROFS gate + copy-up promotion
//! included) handles the group-writable path (e.g. a mode-0664 file touched
//! by a group member), and the ownership/`CAP_FOWNER` fallback handles the
//! mode-000-owner path; the fallback re-runs the mutating admission with
//! `Permission::empty()` before delegating, so the EROFS gate and the
//! copy-up promotion still run on the owner path and lower layers are never
//! written directly. The explicit real permission stage already ran inside
//! `check_real_permission`: ext2/ramfs metadata setters do **not**
//! self-evaluate, so the forward must not skip the gate, and local admission
//! is the only other gate for these entries (the `chmod`/`chown` syscalls
//! perform no VFS-level pre-check — verified `chmod.rs`/`chown.rs`).
//!
//! # Ownership gate
//!
//! The three ownership-sensitive setters (`set_mode`/`set_owner`/
//! `set_group`) additionally run a LOCAL ownership/capability gate before the
//! uniform mutating admission — a deliberate deviation from a
//! `MAY_WRITE`-only shape. The VFS (`sys_chmod`/`sys_chown`) performs no
//! owner/CAP_FOWNER/CAP_CHOWN pre-check and the overlay's real stage runs
//! under the mount creator's (root) credentials, so this gate is the last
//! line: `chmod` requires `fsuid == projected uid` or `CAP_FOWNER` (plus
//! `S_ISUID`/`S_ISGID` masking without `CAP_FSETID`); `chown` requires
//! `CAP_CHOWN`; `chgrp` requires `CAP_CHOWN` or owner-plus-own-group (Linux
//! `inode_owner_or_capable`/`chmod_common`/`setattr_prepare` semantics). The
//! three setters admit with `Permission::empty()` (NOT `MAY_WRITE`), so the
//! ownership/capability gates are authoritative — Linux requires only
//! ownership/capability for chmod/chown, never write access (the
//! chmod-000-then-chmod-644 owner idiom must not fail `EACCES`).
//!
//! The three time setters are infallible VFS surfaces (verified
//! `kernel/src/fs/vfs/fs_apis/inode.rs:380-388`), so their mutating shape is
//! best-effort: a local or real failure is a silent no-op at the overlay
//! boundary, because `EROFS` cannot surface through the trait. Read-driven
//! atime updates stay with the copy-up module's `O_NOATIME` delegation; this
//! module never models a read as a copy-up trigger.
//!
//! Lock contract: this module acquires no Overlay lock. The admission surface
//! is lock-free; the only lock progression is inside the copy-up authority
//! promotion (`ensure_upper_authority`, consumed between the two permission
//! stages), and no Overlay lock is ever held across an underlying permission
//! or metadata callback. Authority is re-resolved per call inside
//! `delegate_to_real` (fresh `select_real_inode()`), so a stale lower/upper
//! observation is never reused for the forward.

use core::time::Duration;

use crate::{
    fs::{
        file::{InodeMode, Permission},
        fs_impls::overlayfs::{AccessType, projection::OverlayInode},
        vfs::inode::Inode,
    },
    prelude::*,
    process::{Gid, Uid, credentials::capabilities::CapSet},
};

/// The ownership/capability facts of the current caller against one projected
/// owner.
///
/// Named result type replacing the earlier positional `(bool, bool)` return of
/// the ownership probe: the `is_owner`/`has_cap` pair is the gate decision of the
/// ownership-sensitive setters and is consumed by name at every call site
/// (`set_mode`/`set_owner`/`set_group`). Module-private to `metadata.rs`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CallerOwnerFacts {
    /// `fsuid == projected uid`.
    is_owner: bool,
    /// The caller holds the probed capability (kernel contexts fail open).
    has_cap: bool,
}

impl OverlayInode {
    // Chmod: the ownership gate first — Linux `inode_owner_or_capable` +
    // `chmod_common` (owner or `CAP_FOWNER`; `S_ISUID`/`S_ISGID` masked
    // without ownership or `CAP_FSETID`) — then the `AccessType::Mutating`
    // admission with `Permission::empty()` (Linux requires only
    // ownership/capability for chmod, never write access, so the gate — not
    // `MAY_WRITE` — is authoritative; the EROFS gate and the copy-up
    // promotion still live in the mutating admission), then a
    // creator-credential forward to the current real authority. The projected
    // metadata is fetched once and reused by the two probes (no double
    // fetch).
    pub(in crate::fs::fs_impls::overlayfs) fn set_mode_impl(&self, mode: InodeMode) -> Result<()> {
        let metadata = self.metadata()?;
        let facts = self.caller_owner_facts(metadata.uid, CapSet::FOWNER);
        if !facts.is_owner && !facts.has_cap {
            return Err(Error::with_message(
                Errno::EPERM,
                "the caller is not the file owner and lacks CAP_FOWNER",
            ));
        }
        // Linux `chmod_common`: a non-owner without `CAP_FSETID` cannot
        // stamp set-id bits onto the file.
        let has_fsetid = self
            .caller_owner_facts(metadata.uid, CapSet::FSETID)
            .has_cap;
        let mut mode = mode;
        if !facts.is_owner && !has_fsetid {
            mode.remove(InodeMode::S_ISUID | InodeMode::S_ISGID);
        }
        self.check_permission(AccessType::Mutating, Permission::empty())?;
        self.delegate_to_real(|real| real.set_mode(mode))
    }

    // Chown uid: the ownership gate first — Linux
    // `chown_common`/`setattr_prepare`: an ownership change requires
    // `CAP_CHOWN`; a no-op chown to the same projected owner is exempt —
    // then the `Permission::empty()` mutating admission and a
    // creator-credential forward.
    pub(in crate::fs::fs_impls::overlayfs) fn set_owner_impl(&self, uid: Uid) -> Result<()> {
        let metadata = self.metadata()?;
        if uid != metadata.uid && !self.caller_owner_facts(metadata.uid, CapSet::CHOWN).has_cap {
            return Err(Error::with_message(
                Errno::EPERM,
                "the caller lacks CAP_CHOWN for an ownership change",
            ));
        }
        self.check_permission(AccessType::Mutating, Permission::empty())?;
        self.delegate_to_real(|real| real.set_owner(uid))
    }

    // Chown gid: the ownership gate first — Linux
    // `chown_common`/`setattr_prepare`: a group change requires `CAP_CHOWN`,
    // or — the owner's `chgrp` to one of its own groups — ownership plus
    // target-group membership; a no-op chgrp to the same projected group is
    // exempt — then the `Permission::empty()` mutating admission and a
    // creator-credential forward.
    pub(in crate::fs::fs_impls::overlayfs) fn set_group_impl(&self, gid: Gid) -> Result<()> {
        let metadata = self.metadata()?;
        if gid != metadata.gid {
            let facts = self.caller_owner_facts(metadata.uid, CapSet::CHOWN);
            // The owner-chgrp exemption (Linux `in_group_p`): the owner may
            // change the group to one of its own supplementary groups —
            // membership probed through the shared current-groups accessor
            // (kernel contexts default to `false`).
            let in_own_group = OverlayInode::current_in_group(gid);
            if !facts.has_cap && !(facts.is_owner && in_own_group) {
                return Err(Error::with_message(
                    Errno::EPERM,
                    "the caller lacks CAP_CHOWN for a group change",
                ));
            }
        }
        self.check_permission(AccessType::Mutating, Permission::empty())?;
        self.delegate_to_real(|real| real.set_group(gid))
    }

    // Utimens atime: best-effort time setter — the trait surface is
    // infallible, so an admission or delegation failure is a silent no-op at
    // the overlay boundary (the `EROFS`-surfacing gap is a known VFS
    // dependency). Read-driven atime updates never reach this entry; they
    // stay with the copy-up module's `O_NOATIME` delegation.
    pub(in crate::fs::fs_impls::overlayfs) fn set_atime_impl(&self, time: Duration) {
        self.best_effort_time_set(|real| real.set_atime(time));
    }

    // Utimens mtime: best-effort time setter; same shape as `set_atime`
    // (shared `best_effort_time_set` composition).
    pub(in crate::fs::fs_impls::overlayfs) fn set_mtime_impl(&self, time: Duration) {
        self.best_effort_time_set(|real| real.set_mtime(time));
    }

    // Utimens ctime: best-effort time setter; same shape as `set_atime`
    // (shared `best_effort_time_set` composition).
    pub(in crate::fs::fs_impls::overlayfs) fn set_ctime_impl(&self, time: Duration) {
        self.best_effort_time_set(|real| real.set_ctime(time));
    }
}

impl OverlayInode {
    /// Resolves the caller's ownership/capability facts against the given
    /// projected owner.
    ///
    /// The projected `uid` is passed in so the caller's single
    /// `metadata()` snapshot is reused (no double fetch): `is_owner` =
    /// `fsuid == projected_uid` via the shared `current_fsuid()` helper;
    /// `has_cap` = the caller holds `cap`, via the shared
    /// `current_task_has_capability` probe (`permission.rs` — kernel contexts
    /// fail open). Consumed by the three ownership-sensitive setters and the
    /// best-effort time-setter gate (five probes across the four gates).
    fn caller_owner_facts(&self, projected_uid: Uid, cap: CapSet) -> CallerOwnerFacts {
        let is_owner = OverlayInode::current_fsuid().is_some_and(|fsuid| fsuid == projected_uid);
        CallerOwnerFacts {
            is_owner,
            has_cap: OverlayInode::current_task_has_capability(cap),
        }
    }

    /// Runs one best-effort time setter.
    ///
    /// The three infallible time-setter trait methods share one shape — the
    /// Linux utimensat admission disjunction, then
    /// `let _ = self.delegate_to_real(...)` wrapping a one-line real call;
    /// this private helper is the single composition (three call sites within
    /// this module's execution paths). Linux utimensat admits write access OR
    /// ownership OR `CAP_FOWNER`: the `Permission::MAY_WRITE` mutating
    /// admission (EROFS gate + copy-up promotion included) handles the
    /// write-access paths — e.g. a mode-0664 file touched by a group member —
    /// and the ownership/`CAP_FOWNER` fallback handles the mode-000-owner
    /// path. The owner/`CAP_FOWNER` fallback re-runs the mutating admission
    /// with `Permission::empty()` BEFORE delegating, so the EROFS gate and
    /// the copy-up promotion still run on the fallback path and lower layers
    /// are never written directly. Only when the `MAY_WRITE` admission fails
    /// AND the caller is neither owner nor `CAP_FOWNER` — or the empty-perm
    /// fallback admission itself fails (EROFS) — is the update silently
    /// dropped; a local or real failure is a silent no-op at the overlay
    /// boundary, because `EROFS` cannot surface through the infallible trait
    /// surface (known VFS dependency).
    fn best_effort_time_set(&self, operation_fn: impl FnOnce(&Arc<dyn Inode>)) {
        let Some(metadata) = self.metadata().ok() else {
            return;
        };
        let facts = self.caller_owner_facts(metadata.uid, CapSet::FOWNER);
        if self
            .check_permission(AccessType::Mutating, Permission::MAY_WRITE)
            .is_err()
        {
            if !facts.is_owner && !facts.has_cap {
                return;
            }
            // Owner / CAP_FOWNER fallback: run the mutating admission with
            // `Permission::empty()` so the EROFS gate and the copy-up
            // promotion still run before delegation — lower layers are never
            // written directly.
            if self
                .check_permission(AccessType::Mutating, Permission::empty())
                .is_err()
            {
                return;
            }
        }
        let _ = self.delegate_to_real(|real| {
            operation_fn(real);
            Ok(())
        });
    }
}
