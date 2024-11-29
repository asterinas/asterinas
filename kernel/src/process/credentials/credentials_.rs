// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard};

use super::{group::AtomicGid, user::AtomicUid, Gid, Uid};
use crate::{
    prelude::*,
    process::credentials::capabilities::{AtomicCapSet, CapSet},
};

#[derive(Debug)]
pub(super) struct Credentials_ {
    /// Real user id. The user to which the process belongs.
    ruid: AtomicUid,
    /// Effective user id. Used to determine the permissions granted to a process when it tries to perform various operations (i.e., system calls)
    euid: AtomicUid,
    /// Saved-set uid. Used by set_uid elf, the saved_set_uid will be set if the elf has setuid bit
    suid: AtomicUid,
    /// User id used for filesystem checks.
    fsuid: AtomicUid,

    /// Real group id. The group to which the process belongs
    rgid: AtomicGid,
    /// Effective gid,
    egid: AtomicGid,
    /// Saved-set gid. Used by set_gid elf, the saved_set_gid will be set if the elf has setgid bit
    sgid: AtomicGid,
    /// Group id used for file system checks.
    fsgid: AtomicGid,

    /// A set of additional groups to which a process belongs.
    supplementary_gids: RwLock<BTreeSet<Gid>>,

    /// The Linux capabilities.
    ///
    /// This is not the capability (in static_cap.rs) enforced on rust objects.
    /// Capability that child processes can inherit
    inheritable_capset: AtomicCapSet,

    /// Capabilities that a process can potentially be granted.
    /// It defines the maximum set of privileges that the process could possibly have.
    /// Even if the process is not currently using these privileges, it has the potential ability to enable them.
    permitted_capset: AtomicCapSet,

    /// Capability that we can actually use
    effective_capset: AtomicCapSet,
}

impl Credentials_ {
    /// Create a new credentials. ruid, euid, suid will be set as the same uid, and gid is the same.
    pub fn new(uid: Uid, gid: Gid, capset: CapSet) -> Self {
        let mut supplementary_gids = BTreeSet::new();
        supplementary_gids.insert(gid);

        Self {
            ruid: AtomicUid::new(uid),
            euid: AtomicUid::new(uid),
            suid: AtomicUid::new(uid),
            fsuid: AtomicUid::new(uid),
            rgid: AtomicGid::new(gid),
            egid: AtomicGid::new(gid),
            sgid: AtomicGid::new(gid),
            fsgid: AtomicGid::new(gid),
            supplementary_gids: RwLock::new(supplementary_gids),
            inheritable_capset: AtomicCapSet::new(capset),
            permitted_capset: AtomicCapSet::new(capset),
            effective_capset: AtomicCapSet::new(capset),
        }
    }

    fn is_privileged(&self) -> bool {
        self.euid.is_root()
    }

    //  ******* Uid methods *******

    pub(super) fn ruid(&self) -> Uid {
        self.ruid.load(Ordering::Relaxed)
    }

    pub(super) fn euid(&self) -> Uid {
        self.euid.load(Ordering::Relaxed)
    }

    pub(super) fn suid(&self) -> Uid {
        self.suid.load(Ordering::Relaxed)
    }

    pub(super) fn fsuid(&self) -> Uid {
        self.fsuid.load(Ordering::Relaxed)
    }

    pub(super) fn set_uid(&self, uid: Uid) {
        if self.is_privileged() {
            self.ruid.store(uid, Ordering::Relaxed);
            self.euid.store(uid, Ordering::Relaxed);
            self.suid.store(uid, Ordering::Relaxed);
            self.fsuid.store(uid, Ordering::Relaxed);
        } else {
            self.euid.store(uid, Ordering::Relaxed);
            self.fsuid.store(uid, Ordering::Relaxed);
        }
    }

    pub(super) fn set_reuid(&self, ruid: Option<Uid>, euid: Option<Uid>) -> Result<()> {
        self.check_uid_perm(ruid.as_ref(), euid.as_ref(), None, false)?;

        let should_set_suid = ruid.is_some() || euid.is_some_and(|euid| euid != self.ruid());

        self.set_resuid_unchecked(ruid, euid, None);

        if should_set_suid {
            self.suid.store(self.euid(), Ordering::Release);
        }

        // FIXME: should we set fsuid here? The linux document for syscall `setfsuid` is contradictory
        // with the document of syscall `setreuid`. The `setfsuid` document says the `fsuid` is always
        // the same as `euid`, but `setreuid` does not mention the `fsuid` should be set.
        self.fsuid.store(self.euid(), Ordering::Release);

        Ok(())
    }

    pub(super) fn set_resuid(
        &self,
        ruid: Option<Uid>,
        euid: Option<Uid>,
        suid: Option<Uid>,
    ) -> Result<()> {
        self.check_uid_perm(ruid.as_ref(), euid.as_ref(), suid.as_ref(), true)?;

        self.set_resuid_unchecked(ruid, euid, suid);

        self.fsuid.store(self.euid(), Ordering::Release);

        Ok(())
    }

    pub(super) fn set_fsuid(&self, fsuid: Option<Uid>) -> Result<Uid> {
        let old_fsuid = self.fsuid();

        let Some(fsuid) = fsuid else {
            return Ok(old_fsuid);
        };

        if self.is_privileged() {
            self.fsuid.store(fsuid, Ordering::Release);
            return Ok(old_fsuid);
        }

        if fsuid != self.ruid() && fsuid != self.euid() && fsuid != self.suid() {
            return_errno_with_message!(
                Errno::EPERM,
                "fsuid can only be one of old ruid, old euid and old suid."
            )
        }

        self.fsuid.store(fsuid, Ordering::Release);

        Ok(old_fsuid)
    }

    pub(super) fn set_euid(&self, euid: Uid) {
        self.euid.store(euid, Ordering::Release);
    }

    pub(super) fn set_suid(&self, suid: Uid) {
        self.suid.store(suid, Ordering::Release);
    }

    // For `setreuid`, ruid can *NOT* be set to old suid,
    // while for `setresuid`, ruid can be set to old suid.
    fn check_uid_perm(
        &self,
        ruid: Option<&Uid>,
        euid: Option<&Uid>,
        suid: Option<&Uid>,
        ruid_may_be_old_suid: bool,
    ) -> Result<()> {
        if self.is_privileged() {
            return Ok(());
        }

        if let Some(ruid) = ruid
            && *ruid != self.ruid()
            && *ruid != self.euid()
            && (!ruid_may_be_old_suid || *ruid != self.suid())
        {
            return_errno_with_message!(
                Errno::EPERM,
                "ruid can only be one of old ruid, old euid (and old suid)."
            );
        }

        if let Some(euid) = euid
            && *euid != self.ruid()
            && *euid != self.euid()
            && *euid != self.suid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "euid can only be one of old ruid, old euid and old suid."
            )
        }

        if let Some(suid) = suid
            && *suid != self.ruid()
            && *suid != self.euid()
            && *suid != self.suid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "suid can only be one of old ruid, old euid and old suid."
            )
        }

        Ok(())
    }

    fn set_resuid_unchecked(&self, ruid: Option<Uid>, euid: Option<Uid>, suid: Option<Uid>) {
        if let Some(ruid) = ruid {
            self.ruid.store(ruid, Ordering::Relaxed);
        }

        if let Some(euid) = euid {
            self.euid.store(euid, Ordering::Relaxed);
        }

        if let Some(suid) = suid {
            self.suid.store(suid, Ordering::Relaxed);
        }
    }

    //  ******* Gid methods *******

    pub(super) fn rgid(&self) -> Gid {
        self.rgid.load(Ordering::Relaxed)
    }

    pub(super) fn egid(&self) -> Gid {
        self.egid.load(Ordering::Relaxed)
    }

    pub(super) fn sgid(&self) -> Gid {
        self.sgid.load(Ordering::Relaxed)
    }

    pub(super) fn fsgid(&self) -> Gid {
        self.fsgid.load(Ordering::Relaxed)
    }

    pub(super) fn set_gid(&self, gid: Gid) {
        if self.is_privileged() {
            self.rgid.store(gid, Ordering::Relaxed);
            self.egid.store(gid, Ordering::Relaxed);
            self.sgid.store(gid, Ordering::Relaxed);
            self.fsgid.store(gid, Ordering::Relaxed);
        } else {
            self.egid.store(gid, Ordering::Relaxed);
            self.fsgid.store(gid, Ordering::Relaxed);
        }
    }

    pub(super) fn set_regid(&self, rgid: Option<Gid>, egid: Option<Gid>) -> Result<()> {
        self.check_gid_perm(rgid.as_ref(), egid.as_ref(), None, false)?;

        let should_set_sgid = rgid.is_some() || egid.is_some_and(|egid| egid != self.rgid());

        self.set_resgid_unchecked(rgid, egid, None);

        if should_set_sgid {
            self.sgid.store(self.egid(), Ordering::Relaxed);
        }

        self.fsgid.store(self.egid(), Ordering::Relaxed);

        Ok(())
    }

    pub(super) fn set_resgid(
        &self,
        rgid: Option<Gid>,
        egid: Option<Gid>,
        sgid: Option<Gid>,
    ) -> Result<()> {
        self.check_gid_perm(rgid.as_ref(), egid.as_ref(), sgid.as_ref(), true)?;

        self.set_resgid_unchecked(rgid, egid, sgid);

        self.fsgid.store(self.egid(), Ordering::Relaxed);

        Ok(())
    }

    pub(super) fn set_fsgid(&self, fsgid: Option<Gid>) -> Result<Gid> {
        let old_fsgid = self.fsgid();

        let Some(fsgid) = fsgid else {
            return Ok(old_fsgid);
        };

        if self.is_privileged() {
            self.fsgid.store(fsgid, Ordering::Relaxed);
            return Ok(old_fsgid);
        }

        if fsgid != self.rgid() && fsgid != self.egid() && fsgid != self.sgid() {
            return_errno_with_message!(
                Errno::EPERM,
                "fsuid can only be one of old ruid, old euid and old suid."
            )
        }

        self.fsgid.store(fsgid, Ordering::Relaxed);

        Ok(old_fsgid)
    }

    pub(super) fn set_egid(&self, egid: Gid) {
        self.egid.store(egid, Ordering::Relaxed);
    }

    pub(super) fn set_sgid(&self, sgid: Gid) {
        self.sgid.store(sgid, Ordering::Relaxed);
    }

    // For `setregid`, rgid can *NOT* be set to old sgid,
    // while for `setresgid`, ruid can be set to old sgid.
    fn check_gid_perm(
        &self,
        rgid: Option<&Gid>,
        egid: Option<&Gid>,
        sgid: Option<&Gid>,
        rgid_may_be_old_sgid: bool,
    ) -> Result<()> {
        if self.is_privileged() {
            return Ok(());
        }

        if let Some(rgid) = rgid
            && *rgid != self.rgid()
            && *rgid != self.egid()
            && (!rgid_may_be_old_sgid || *rgid != self.sgid())
        {
            return_errno_with_message!(
                Errno::EPERM,
                "rgid can only be one of old rgid, old egid (and old sgid)."
            );
        }

        if let Some(egid) = egid
            && *egid != self.rgid()
            && *egid != self.egid()
            && *egid != self.sgid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "egid can only be one of old rgid, old egid and old sgid."
            )
        }

        if let Some(sgid) = sgid
            && *sgid != self.rgid()
            && *sgid != self.egid()
            && *sgid != self.sgid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "sgid can only be one of old rgid, old egid and old sgid."
            )
        }

        Ok(())
    }

    fn set_resgid_unchecked(&self, rgid: Option<Gid>, egid: Option<Gid>, sgid: Option<Gid>) {
        if let Some(rgid) = rgid {
            self.rgid.store(rgid, Ordering::Relaxed);
        }

        if let Some(egid) = egid {
            self.egid.store(egid, Ordering::Relaxed);
        }

        if let Some(sgid) = sgid {
            self.sgid.store(sgid, Ordering::Relaxed);
        }
    }

    //  ******* Supplementary groups methods *******

    pub(super) fn groups(&self) -> RwLockReadGuard<BTreeSet<Gid>, PreemptDisabled> {
        self.supplementary_gids.read()
    }

    pub(super) fn groups_mut(&self) -> RwLockWriteGuard<BTreeSet<Gid>, PreemptDisabled> {
        self.supplementary_gids.write()
    }

    //  ******* Linux Capability methods *******

    pub(super) fn inheritable_capset(&self) -> CapSet {
        self.inheritable_capset.load(Ordering::Relaxed)
    }

    pub(super) fn permitted_capset(&self) -> CapSet {
        self.permitted_capset.load(Ordering::Relaxed)
    }

    pub(super) fn effective_capset(&self) -> CapSet {
        self.effective_capset.load(Ordering::Relaxed)
    }

    pub(super) fn set_inheritable_capset(&self, inheritable_capset: CapSet) {
        self.inheritable_capset
            .store(inheritable_capset, Ordering::Relaxed);
    }

    pub(super) fn set_permitted_capset(&self, permitted_capset: CapSet) {
        self.permitted_capset
            .store(permitted_capset, Ordering::Relaxed);
    }

    pub(super) fn set_effective_capset(&self, effective_capset: CapSet) {
        self.effective_capset
            .store(effective_capset, Ordering::Relaxed);
    }
}

impl Clone for Credentials_ {
    fn clone(&self) -> Self {
        Self {
            ruid: self.ruid.clone(),
            euid: self.euid.clone(),
            suid: self.suid.clone(),
            fsuid: self.fsuid.clone(),
            rgid: self.rgid.clone(),
            egid: self.egid.clone(),
            sgid: self.sgid.clone(),
            fsgid: self.fsgid.clone(),
            supplementary_gids: RwLock::new(self.supplementary_gids.read().clone()),
            inheritable_capset: self.inheritable_capset.clone(),
            permitted_capset: self.permitted_capset.clone(),
            effective_capset: self.effective_capset.clone(),
        }
    }
}
