// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard};

use super::{
    Gid, SecureBits, Uid, group::AtomicGid, secure_bits::AtomicSecureBits, user::AtomicUid,
};
use crate::{
    prelude::*,
    process::credentials::capabilities::{AtomicCapSet, CapSet},
};

#[derive(Debug)]
pub(super) struct Credentials_ {
    /// The real user ID.
    ///
    /// This is the user to which the process belongs.
    ruid: AtomicUid,
    /// The effective user ID.
    ///
    /// This is used to determine the permissions granted to a process when it tries to perform
    /// various operations (e.g., system calls).
    euid: AtomicUid,
    /// The saved-set user ID.
    ///
    /// This saves a copy of the effective user ID that were set when the program was executed.
    suid: AtomicUid,
    /// The filesystem user ID.
    ///
    /// This is used to determine permissions for accessing files.
    fsuid: AtomicUid,

    /// The real group ID.
    ///
    /// This is the group to which the process belongs.
    rgid: AtomicGid,
    /// The effective group ID.
    ///
    /// This is used to determine the permissions granted to a process when it tries to perform
    /// various operations (e.g., system calls).
    egid: AtomicGid,
    /// The saved-set group ID.
    ///
    /// This saves a copy of the effective group ID that were set when the program was executed.
    sgid: AtomicGid,
    /// The filesystem group ID.
    ///
    /// This is used to determine permissions for accessing files.
    fsgid: AtomicGid,

    /// A set of additional groups to which a process belongs.
    supplementary_gids: RwLock<BTreeSet<Gid>>,

    // The Linux capabilities. They are not the capability (in `static_cap.rs`) that is enforced on
    // Rust objects.
    //
    /// Capabilities that child processes can inherit.
    inheritable_capset: AtomicCapSet,
    /// Capabilities that a process can potentially be granted.
    ///
    /// It defines the maximum set of privileges that the process could possibly have. Even if the
    /// process is not currently using these privileges, it has the potential ability to enable
    /// them.
    permitted_capset: AtomicCapSet,
    /// Capabilities that we can actually use.
    effective_capset: AtomicCapSet,

    /// Secure bits.
    securebits: AtomicSecureBits,
}

impl Credentials_ {
    /// Creates a new `Credentials_`.
    ///
    /// The real, effective, saved set, and filesystem IDs will be initialized to the same ID.
    pub(super) fn new(uid: Uid, gid: Gid, capset: CapSet) -> Self {
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
            securebits: AtomicSecureBits::new(SecureBits::new_empty()),
        }
    }

    //  ******* UID methods *******

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

    pub(super) fn set_uid(&self, uid: Uid) -> Result<()> {
        if self.effective_capset().contains(CapSet::SETUID) {
            self.set_resuid_unchecked(Some(uid), Some(uid), Some(uid));
            Ok(())
        } else {
            self.set_resuid(None, Some(uid), None)
        }
    }

    pub(super) fn set_reuid(&self, ruid: Option<Uid>, euid: Option<Uid>) -> Result<()> {
        self.check_uid_perm(ruid.as_ref(), euid.as_ref(), None, false)?;

        let should_set_suid = ruid.is_some() || euid.is_some_and(|euid| euid != self.ruid());
        let suid = if should_set_suid {
            Some(euid.unwrap_or_else(|| self.euid()))
        } else {
            None
        };
        self.set_resuid_unchecked(ruid, euid, suid);

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

        Ok(())
    }

    pub(super) fn set_fsuid(&self, fsuid: Option<Uid>) -> core::result::Result<Uid, Uid> {
        let old_fsuid = self.fsuid();

        let Some(fsuid) = fsuid else {
            return Ok(old_fsuid);
        };

        if self.effective_capset().contains(CapSet::SETUID) {
            self.set_fsuid_unchecked(fsuid);
            return Ok(old_fsuid);
        }

        if fsuid != self.ruid() && fsuid != self.euid() && fsuid != self.suid() {
            // The new filesystem UID is not one of the associated UIDs.
            return Err(old_fsuid);
        }

        self.set_fsuid_unchecked(fsuid);

        Ok(old_fsuid)
    }

    pub(super) fn set_euid(&self, euid: Uid) {
        self.set_resuid_unchecked(None, Some(euid), None);
    }

    pub(super) fn set_suid(&self, suid: Uid) {
        self.set_resuid_unchecked(None, None, Some(suid));
    }

    // For `setreuid`, the real UID can *NOT* be set to the old saved-set user ID,
    // For `setresuid`, the real UID can be set to the old saved-set user ID.
    fn check_uid_perm(
        &self,
        ruid: Option<&Uid>,
        euid: Option<&Uid>,
        suid: Option<&Uid>,
        ruid_may_be_old_suid: bool,
    ) -> Result<()> {
        if self.effective_capset().contains(CapSet::SETUID) {
            return Ok(());
        }

        if let Some(ruid) = ruid
            && *ruid != self.ruid()
            && *ruid != self.euid()
            && (!ruid_may_be_old_suid || *ruid != self.suid())
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the new real UID is not one of the associated UIDs"
            );
        }

        if let Some(euid) = euid
            && *euid != self.ruid()
            && *euid != self.euid()
            && *euid != self.suid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the new effective UID is not one of the associated UIDs"
            )
        }

        if let Some(suid) = suid
            && *suid != self.ruid()
            && *suid != self.euid()
            && *suid != self.suid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the new saved-set UID is not one of the associated UIDs"
            )
        }

        Ok(())
    }

    fn set_resuid_unchecked(&self, ruid: Option<Uid>, euid: Option<Uid>, suid: Option<Uid>) {
        let old_ruid = self.ruid();
        let old_euid = self.euid();
        let old_suid = self.suid();

        let new_ruid = if let Some(ruid) = ruid {
            self.ruid.store(ruid, Ordering::Relaxed);
            ruid
        } else {
            old_ruid
        };

        let new_euid = if let Some(euid) = euid {
            self.euid.store(euid, Ordering::Relaxed);
            euid
        } else {
            old_euid
        };

        let new_suid = if let Some(suid) = suid {
            self.suid.store(suid, Ordering::Relaxed);
            suid
        } else {
            old_suid
        };

        self.set_fsuid_unchecked(new_euid);

        // If the `SECBIT_NO_SETUID_FIXUP` bit is set, do not adjust capabilities.
        // Reference: The "SECBIT_NO_SETUID_FIXUP" section in
        // <https://man7.org/linux/man-pages/man7/capabilities.7.html>.
        if self.securebits().no_setuid_fixup() {
            return;
        }

        // Begin to adjust capabilities.
        // Reference: The "Effect of user ID changes on capabilities" section in
        // <https://man7.org/linux/man-pages/man7/capabilities.7.html>.

        let had_root = old_ruid.is_root() || old_euid.is_root() || old_suid.is_root();
        let all_nonroot = !new_ruid.is_root() && !new_euid.is_root() && !new_suid.is_root();
        if had_root && all_nonroot && !self.keep_capabilities() {
            self.set_permitted_capset(CapSet::empty());
            self.set_inheritable_capset(CapSet::empty());
            // TODO: Clear ambient capabilities when we support it. Note that ambient capabilities
            // should be cleared even if `keep_capabilities` is true.
        }

        if old_euid.is_root() && !new_euid.is_root() {
            self.set_effective_capset(CapSet::empty());
        } else if !old_euid.is_root() && new_euid.is_root() {
            let permitted = self.permitted_capset();
            self.set_effective_capset(permitted);
        }
    }

    fn set_fsuid_unchecked(&self, fsuid: Uid) {
        let old_fsuid = self.fsuid();
        self.fsuid.store(fsuid, Ordering::Relaxed);

        if old_fsuid.is_root() && !fsuid.is_root() {
            // Reference: The "Effect of user ID changes on capabilities" section in
            // <https://man7.org/linux/man-pages/man7/capabilities.7.html>.
            let cap_to_remove = CapSet::CHOWN
                | CapSet::DAC_OVERRIDE
                | CapSet::FOWNER
                | CapSet::DAC_READ_SEARCH
                | CapSet::FSETID
                | CapSet::LINUX_IMMUTABLE
                | CapSet::MAC_OVERRIDE
                | CapSet::MKNOD;
            let old_cap = self.effective_capset();
            self.set_effective_capset(old_cap - cap_to_remove);
        }
    }

    //  ******* GID methods *******

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

    pub(super) fn set_gid(&self, gid: Gid) -> Result<()> {
        if self.effective_capset().contains(CapSet::SETGID) {
            self.set_resgid_unchecked(Some(gid), Some(gid), Some(gid));
            Ok(())
        } else {
            self.set_resgid(None, Some(gid), None)
        }
    }

    pub(super) fn set_regid(&self, rgid: Option<Gid>, egid: Option<Gid>) -> Result<()> {
        self.check_gid_perm(rgid.as_ref(), egid.as_ref(), None, false)?;

        let should_set_sgid = rgid.is_some() || egid.is_some_and(|egid| egid != self.rgid());
        let sgid = if should_set_sgid {
            Some(egid.unwrap_or_else(|| self.egid()))
        } else {
            None
        };
        self.set_resgid_unchecked(rgid, egid, sgid);

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

        Ok(())
    }

    pub(super) fn set_fsgid(&self, fsgid: Option<Gid>) -> core::result::Result<Gid, Gid> {
        let old_fsgid = self.fsgid();

        let Some(fsgid) = fsgid else {
            return Ok(old_fsgid);
        };

        if fsgid == old_fsgid {
            return Ok(old_fsgid);
        }

        if self.effective_capset().contains(CapSet::SETGID) {
            self.set_fsgid_unchecked(fsgid);
            return Ok(old_fsgid);
        }

        if fsgid != self.rgid() && fsgid != self.egid() && fsgid != self.sgid() {
            // The new filesystem GID is not one of the associated GIDs.
            return Err(old_fsgid);
        }

        self.set_fsgid_unchecked(fsgid);

        Ok(old_fsgid)
    }

    pub(super) fn set_egid(&self, egid: Gid) {
        self.set_resgid_unchecked(None, Some(egid), None);
    }

    pub(super) fn set_sgid(&self, sgid: Gid) {
        self.set_resgid_unchecked(None, None, Some(sgid));
    }

    // For `setregid`, the real GID can *NOT* be set to old saved-set GID,
    // For `setresgid`, the real GID can be set to the old saved-set GID.
    fn check_gid_perm(
        &self,
        rgid: Option<&Gid>,
        egid: Option<&Gid>,
        sgid: Option<&Gid>,
        rgid_may_be_old_sgid: bool,
    ) -> Result<()> {
        if self.effective_capset().contains(CapSet::SETGID) {
            return Ok(());
        }

        if let Some(rgid) = rgid
            && *rgid != self.rgid()
            && *rgid != self.egid()
            && (!rgid_may_be_old_sgid || *rgid != self.sgid())
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the new real GID is not one of the associated GIDs"
            );
        }

        if let Some(egid) = egid
            && *egid != self.rgid()
            && *egid != self.egid()
            && *egid != self.sgid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the new effective GID is not one of the associated GIDs"
            )
        }

        if let Some(sgid) = sgid
            && *sgid != self.rgid()
            && *sgid != self.egid()
            && *sgid != self.sgid()
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the new saved-set GID is not one of the associated GIDs"
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

        self.set_fsgid_unchecked(self.egid());
    }

    fn set_fsgid_unchecked(&self, fsuid: Gid) {
        self.fsgid.store(fsuid, Ordering::Relaxed);
    }

    //  ******* Supplementary Groups methods *******

    pub(super) fn groups(&self) -> RwLockReadGuard<'_, BTreeSet<Gid>, PreemptDisabled> {
        self.supplementary_gids.read()
    }

    pub(super) fn groups_mut(&self) -> RwLockWriteGuard<'_, BTreeSet<Gid>, PreemptDisabled> {
        self.supplementary_gids.write()
    }

    //  ******* Linux Capabilities methods *******

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

    pub(super) fn keep_capabilities(&self) -> bool {
        self.securebits.load(Ordering::Relaxed).keep_capabilities()
    }

    pub(super) fn set_keep_capabilities(&self, keep_capabilities: bool) -> Result<()> {
        let current_bits = self.securebits();
        let stored_bits = if !keep_capabilities {
            current_bits - SecureBits::KEEP_CAPS
        } else {
            current_bits | SecureBits::KEEP_CAPS
        };

        self.securebits.try_store(stored_bits, Ordering::Relaxed)
    }

    //  ******* Secure Bits methods *******

    pub(super) fn securebits(&self) -> SecureBits {
        self.securebits.load(Ordering::Relaxed)
    }

    pub(super) fn set_securebits(&self, securebits: SecureBits) -> Result<()> {
        if !self.effective_capset().contains(CapSet::SETPCAP) {
            return_errno_with_message!(
                Errno::EPERM,
                "only threads with CAP_SETPCAP can change secure bits"
            );
        }

        self.securebits.try_store(securebits, Ordering::Relaxed)
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
            securebits: self.securebits.clone(),
        }
    }
}
