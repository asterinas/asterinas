// SPDX-License-Identifier: MPL-2.0

//! The two-stage permission admission pipeline.
//!
//! This module hosts the admission surface: the single entry
//! [`OverlayInode::check_permission`] (the two stages split into two private
//! helpers — the copy-up sits between the stages, so a fused pipeline is
//! wrong), the lock-free local stage
//! [`OverlayInode::check_local_permission`] (EROFS gate for the `Mutating`
//! class + the projected-DAC block), the real-handle stage
//! [`OverlayInode::check_real_permission`] (the explicit real re-check under
//! the creator-credential scope; the copy-up promotion lives in the entry,
//! not here). The canonical read-only `Inode::check_permission` forwarder
//! lives in `projection/inode.rs`; it calls this two-parameter inherent
//! admission entry with `AccessType::ReadOnly` and never promotes.
//!
//! Pipeline: the local stage always runs first and is entirely lock-free.
//! For the `Mutating` class the entry then promotes unconditionally via
//! `ensure_upper_authority()` — the copy-up lives between the stages, and the
//! elevation is independent of the permission skip
//! (`MountPolicy::is_default_permissions`); the real stage
//! [`OverlayInode::check_real_permission`] then evaluates the current real
//! authority (`select_real_inode()`) under the mount's creator-credential
//! scope. `default_permissions` skips only the real/creator-credential
//! re-check, never the local stage and never the promotion. The explicit real
//! check is authoritative for entries whose underlying real ops do not
//! self-evaluate (ext2/ramfs metadata setters) and is a benign double
//! evaluation for xattr ops that self-evaluate under the same scope (kept for
//! gate independence).
//!
//! The local DAC block mirrors the VFS default `Inode::check_permission`
//! algorithm (`kernel/src/fs/vfs/fs_apis/inode.rs:573-640`) against the
//! projected `OverlayInode::metadata()` (mode/uid/gid), with the
//! `DAC_OVERRIDE` reduction via `lsm_hooks::on_capable`. It is inlined here
//! because no reusable kernel helper exists (VFS gap); see the TODO
//! at the Projected-DAC block. The protected-state admission is currently
//! a no-op hook.
//!
//! Lock contract: this module acquires no Overlay lock. The local stage is
//! lock-free (brief `INODE` facts snapshot inside `metadata()`, released
//! before any use); the real stage enters the authority promotion
//! (`DIR -> CUL -> INODE -> WL -> UPPER` order) without holding anything; the
//! creator-credential scope is a task-credential swap, not a lock. No Overlay
//! lock crosses the entry boundary, and no `.unwrap()`/`.expect()` is used
//! anywhere in this security gate.

use ostd::task::{CurrentTask, Task};

use crate::{
    fs::{
        file::Permission,
        fs_impls::overlayfs::{AccessType, projection::OverlayInode},
        vfs::inode::Inode,
    },
    prelude::*,
    process::{
        Gid, Uid,
        credentials::capabilities::CapSet,
        posix_thread::{AsPosixThread, PosixThread},
    },
    security::lsm::hooks as lsm_hooks,
};

impl OverlayInode {
    /// The single admission method: the two-stage permission pipeline every
    /// projected-object request funnels through.
    ///
    /// The local stage (lock-free) always runs first and may reject with
    /// `EROFS` (mutating class on an effective read-only mount) or `EACCES`
    /// (projected-DAC demand denied) with no real handle and no copy-up/
    /// workdir/temp/upper side effect. For the `Mutating` class the entry
    /// then promotes unconditionally via `ensure_upper_authority()` (the
    /// copy-up lives between the stages) — the elevation is independent of
    /// the `default_permissions` skip. Unless the mount was created with
    /// `default_permissions`, the real stage then re-evaluates the current
    /// real authority under the creator-credential scope. The
    /// `default_permissions` skip omits only the real/creator-credential
    /// re-check, never the promotion and never the local stage. A real-stage
    /// failure propagates as-is with no invented rollback (the authority
    /// promotion owns any already-started transition cleanup). Verdicts are
    /// never cached.
    ///
    /// This two-parameter inherent method coexists with the one-parameter
    /// `Inode::check_permission` forwarder in `projection/inode.rs`; Rust
    /// method resolution prefers the inherent method when the arity matches,
    /// so trait callers reach the read-only forwarder and module entries call
    /// this one.
    pub(in crate::fs::fs_impls::overlayfs) fn check_permission(
        &self,
        access: AccessType,
        perm: Permission,
    ) -> Result<()> {
        self.check_local_permission(access, perm)?;
        if access == AccessType::Mutating {
            self.ensure_upper_authority()?;
        }
        if !self.fs_arc()?.policy().is_default_permissions() {
            self.check_real_permission(access, perm)?;
        }
        Ok(())
    }

    /// Runs `operation_fn` with the current task's posix thread.
    ///
    /// The two-step lookup shared by every kernel-context gate of this
    /// module: `Task::current()` then `as_posix_thread()`. The [`CurrentTask`]
    /// guard is passed to `operation_fn` alongside the borrowed
    /// [`PosixThread`] so the guard outlives the borrow. `None` means no
    /// current task or no posix thread (a kernel-internal operation, not a
    /// user process); each caller maps `None` to its own kernel-context
    /// default (fail-open `true` for the capability probe, `None` for the
    /// fsuid probe, fail-closed `false` for the group probe, and the
    /// no-DAC-demand `Ok(())` for the local DAC block).
    fn with_current_posix_thread<T>(
        operation_fn: impl FnOnce(&CurrentTask, &PosixThread) -> T,
    ) -> Option<T> {
        let task = Task::current()?;
        let posix_thread = task.as_posix_thread()?;
        Some(operation_fn(&task, posix_thread))
    }

    /// Returns whether the current task holds `cap` in its user namespace
    /// (the single shared capability probe of this module).
    ///
    /// A process-global probe, so it is an associated function (no `&self`
    /// receiver). Probes through `lsm_hooks::on_capable` with the current
    /// task's posix thread and user namespace (the
    /// `check_local_permission` machinery). Kernel contexts fail open: with
    /// no current task, or no posix thread (a kernel-internal operation,
    /// not a user process), the probe reports `true` — there is no user to
    /// gate (the `check_local_permission` no-task/no-posix-thread
    /// precedent). A user context whose thread-local (and thus user
    /// namespace) is absent reports `false` — fail-closed, since there is
    /// no namespace against which the capability can be scoped. Consumed by
    /// the permission stage (`check_local_permission`, DAC_OVERRIDE) and by
    /// the metadata ownership gates (`metadata.rs`).
    pub(in crate::fs::fs_impls::overlayfs) fn current_task_has_capability(cap: CapSet) -> bool {
        let Some(has_cap) = Self::with_current_posix_thread(|task, posix_thread| {
            task.as_thread_local().is_some_and(|thread_local| {
                let user_ns = thread_local.borrow_user_ns();
                lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
                    user_ns.as_ref(),
                    posix_thread,
                    cap,
                ))
                .is_ok()
            })
        }) else {
            // Kernel contexts fail open: there is no user to gate.
            return true;
        };
        has_cap
    }

    /// Returns the current task's filesystem UID (`None` in a kernel context
    /// — no task / no posix thread).
    ///
    /// Callers treat `None` as "not the owner" (the shared kernel-context
    /// default applied via `is_some_and`). Consumed by
    /// `metadata.rs::caller_owner_facts` and `dir/mod.rs::link`'s source-side
    /// admission.
    pub(in crate::fs::fs_impls::overlayfs) fn current_fsuid() -> Option<Uid> {
        let fsuid =
            Self::with_current_posix_thread(|_, posix_thread| posix_thread.credentials().fsuid())?;
        Some(fsuid)
    }

    /// Returns whether the current task's filesystem group ID or
    /// supplementary group set contains `gid` — Linux `in_group_p` semantics
    /// (`kernel/groups.c` `in_group_p`: `!gid_eq(grp, cred->fsgid)` then
    /// `groups_search(cred->group_info, grp)`).
    ///
    /// Kernel contexts (no task / no posix thread) report `false` — the
    /// shared kernel-context default, applied in one place. The fsgid
    /// disjunct completes the Linux shape: without it, an owner whose
    /// filesystem group ID (`fsgid`) is the target gid but whose
    /// supplementary set omits it would be denied the owner-chgrp exemption.
    /// Consumed by `metadata.rs::set_group`'s owner-chgrp exemption.
    pub(in crate::fs::fs_impls::overlayfs) fn current_in_group(gid: Gid) -> bool {
        let Some(in_group) = Self::with_current_posix_thread(|_, posix_thread| {
            let credentials = posix_thread.credentials();
            gid == credentials.fsgid() || credentials.groups().contains(&gid)
        }) else {
            return false;
        };
        in_group
    }

    /// PRIVATE LOCAL STAGE — the lock-free local half of the two-stage check.
    ///
    /// For the `Mutating` class, the `EROFS` gate (`MountPolicy::
    /// is_effective_read_only`) runs first — before the DAC block — so a
    /// read-only mount fails with no real handle, no copy-up, and no
    /// workdir/temp/upper side effect. The projected-DAC block then mirrors
    /// the VFS default `Inode::check_permission` algorithm
    /// (`inode.rs:573-640`, inlined) against the projected `metadata()`
    /// (mode/uid/gid) and the current task's credentials (`fsuid`/`fsgid`),
    /// with the `DAC_OVERRIDE` reduction via `lsm_hooks::on_capable`.
    /// `Permission::empty()` passes trivially. The protected-state admission
    /// is currently a no-op hook.
    fn check_local_permission(&self, access: AccessType, mut perm: Permission) -> Result<()> {
        // EROFS gate: the mutating class on an effective read-only mount
        // fails before the DAC block and before any authority side effect.
        if access == AccessType::Mutating && self.fs_arc()?.policy().is_effective_read_only() {
            return_errno_with_message!(Errno::EROFS, "the overlay mount is read-only");
        }

        // TODO: this block is a mirror of the VFS default
        // `Inode::check_permission` (`kernel/src/fs/vfs/fs_apis/inode.rs:
        // 573-640`); the VFS exposes no shared mode-DAC evaluator (VFS gap),
        // so the algorithm is inlined here. Once the VFS interface
        // stabilizes, extract a shared `check_mode_dac` and consume it here
        // to eliminate the drift.
        // Projected-DAC block (the `inode.rs:573-640` mirror, inlined). No
        // task / no posix thread / no thread-local: the kernel
        // context is not a user process, so there is no DAC demand to check
        // (mirror's `Option`-based guards, fail-open for non-user contexts;
        // the DAC_OVERRIDE probe is fail-closed when the thread-local is
        // absent — no `.unwrap()`/`.expect()` anywhere in this gate).
        let Some(creds) =
            Self::with_current_posix_thread(|_, posix_thread| posix_thread.credentials())
        else {
            return Ok(());
        };
        let metadata = self.metadata()?;
        let mode = metadata.mode;

        // With DAC_OVERRIDE, read/write DACs are always overridable; the
        // executable DAC is overridable only when at least one exec bit is
        // set (the VFS reduction, `inode.rs:585-604`). The probe runs through
        // the shared user-namespace capability helper: at this point the task
        // and posix thread are known to exist, so the helper's kernel-context
        // fail-open arm is unreachable here and the thread-local-absent case
        // stays fail-closed.
        let has_dac_override = Self::current_task_has_capability(CapSet::DAC_OVERRIDE);
        if has_dac_override {
            perm -= Permission::MAY_READ | Permission::MAY_WRITE;
            if perm.may_exec() {
                if mode.is_owner_executable()
                    || mode.is_group_executable()
                    || mode.is_other_executable()
                {
                    perm -= Permission::MAY_EXEC;
                } else {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "root execute permission denied: no execute bits set"
                    );
                }
            }
        }

        // Owner / group / other mode-DAC checks against the projected
        // metadata (the `inode.rs:606-625` mirror).
        if metadata.uid == creds.fsuid() {
            if (perm.may_read() && !mode.is_owner_readable())
                || (perm.may_write() && !mode.is_owner_writable())
                || (perm.may_exec() && !mode.is_owner_executable())
            {
                return_errno_with_message!(Errno::EACCES, "owner permission check failed");
            }
        } else if metadata.gid == creds.fsgid() {
            if (perm.may_read() && !mode.is_group_readable())
                || (perm.may_write() && !mode.is_group_writable())
                || (perm.may_exec() && !mode.is_group_executable())
            {
                return_errno_with_message!(Errno::EACCES, "group permission check failed");
            }
        } else if (perm.may_read() && !mode.is_other_readable())
            || (perm.may_write() && !mode.is_other_writable())
            || (perm.may_exec() && !mode.is_other_executable())
        {
            return_errno_with_message!(Errno::EACCES, "other permission check failed");
        }

        // Protected-state admission hook: currently a no-op; `protattr`
        // is already classified as overlay-private in the xattr table.
        Ok(())
    }

    /// PRIVATE REAL STAGE — the real-handle half of the two-stage check.
    ///
    /// The copy-up promotion no longer lives here: the entry
    /// [`OverlayInode::check_permission`] already ran `ensure_upper_authority()`
    /// for the `Mutating` class, unconditionally (elevation is independent of
    /// the `default_permissions` skip). This stage only re-resolves the
    /// current real authority per call (`select_real_inode()`) and evaluates
    /// it under the mount's creator-credential scope
    /// (`with_creator_credentials_fn`). The explicit real stage is
    /// authoritative for entries whose underlying ops do not self-evaluate
    /// (metadata setters) and a benign double evaluation for xattr ops that
    /// self-evaluate under the same scope. A failure propagates as-is with no
    /// invented rollback (the authority promotion owns any already-started
    /// transition cleanup/reconcile).
    fn check_real_permission(&self, _access: AccessType, perm: Permission) -> Result<()> {
        let fs = self.fs_arc()?;
        let real = self.select_real_inode();
        fs.policy()
            .credential_policy()
            .with_creator_credentials_fn(|| real.check_permission(perm))
    }
}
