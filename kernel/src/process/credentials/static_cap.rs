// SPDX-License-Identifier: MPL-2.0

use aster_rights::{Dup, Read, TRights, Write};
use aster_rights_proc::require;
use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard};

use super::{Credentials, Gid, SecureBits, Uid, capabilities::CapSet, credentials_::Credentials_};
use crate::prelude::*;

impl<R: TRights> Credentials<R> {
    /// Creates a root `Credentials`.
    ///
    /// This method can only be used when creating the init process.
    pub fn new_root() -> Self {
        let uid = Uid::new_root();
        let gid = Gid::new_root();
        let cap = CapSet::new_root();
        let credentials_ = Arc::new(Credentials_::new(uid, gid, cap));
        Self(credentials_, R::new())
    }

    /// Clones a new `Credentials` from an existing `Credentials`.
    ///
    /// This method requires the `Read` right.
    #[require(R1 > Read)]
    pub fn new_from<R1: TRights>(credentials: &Credentials<R1>) -> Self {
        let credentials_ = Arc::new(credentials.0.as_ref().clone());

        Self(credentials_, R::new())
    }

    /// Duplicates the capabilities.
    ///
    /// This method requires the `Dup` right.
    #[require(R > Dup)]
    pub fn dup(&self) -> Self {
        Self(self.0.clone(), self.1)
    }

    /// Restricts capabilities to a smaller set.
    #[require(R > R1)]
    pub fn restrict<R1: TRights>(self) -> Credentials<R1> {
        let Credentials(credentials_, _) = self;

        Credentials(credentials_, R1::new())
    }

    // *********** UID methods **********

    /// Gets the real user ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn ruid(&self) -> Uid {
        self.0.ruid()
    }

    /// Gets the effective user ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn euid(&self) -> Uid {
        self.0.euid()
    }

    /// Gets the saved-set user ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn suid(&self) -> Uid {
        self.0.suid()
    }

    /// Gets the filesystem user ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn fsuid(&self) -> Uid {
        self.0.fsuid()
    }

    /// Sets the user ID.
    ///
    /// If `self` is privileged, sets the real, effective, saved-set user IDs as `uid`, Otherwise,
    /// sets the effective user ID as `uid`.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_uid(&self, uid: Uid) -> Result<()> {
        self.0.set_uid(uid)
    }

    /// Sets the real, effective user IDs as `ruid`, `euid` respectively.
    ///
    /// If `ruid` or `euid` is `None`, the corresponding user ID will leave unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_reuid(&self, ruid: Option<Uid>, euid: Option<Uid>) -> Result<()> {
        self.0.set_reuid(ruid, euid)
    }

    /// Sets the real, effective, saved-set user IDs as `ruid`, `euid`, `suid` respectively.
    ///
    /// If `ruid`, `euid`, or `suid` is `None`, the corresponding user ID will leave unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_resuid(
        &self,
        ruid: Option<Uid>,
        euid: Option<Uid>,
        suid: Option<Uid>,
    ) -> Result<()> {
        self.0.set_resuid(ruid, euid, suid)
    }

    /// Sets the filesystem user ID as `fsuid` and returns the original filesystem user ID.
    ///
    /// If `fsuid` is `None`, leaves the filesystem user ID unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_fsuid(&self, fsuid: Option<Uid>) -> core::result::Result<Uid, Uid> {
        self.0.set_fsuid(fsuid)
    }

    /// Sets the effective user ID as `euid`.
    ///
    /// This method should only be used when executing a file whose `setuid` bit is set.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_euid(&self, euid: Uid) {
        self.0.set_euid(euid);
    }

    /// Sets the saved-set user ID as the same of the effective user ID.
    ///
    /// This method should only be used when executing a new executable file.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn reset_suid(&self) {
        let euid = self.0.euid();
        self.0.set_suid(euid);
    }

    // *********** GID methods **********

    /// Gets the real group ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn rgid(&self) -> Gid {
        self.0.rgid()
    }

    /// Gets the effective group ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn egid(&self) -> Gid {
        self.0.egid()
    }

    /// Gets the saved-set group ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn sgid(&self) -> Gid {
        self.0.sgid()
    }

    /// Gets the filesystem group ID.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn fsgid(&self) -> Gid {
        self.0.fsgid()
    }

    /// Sets the group ID.
    ///
    /// If `self` is privileged, sets the real, effective, saved-set group IDs as `gid`, Otherwise,
    /// sets the effective group ID as `gid`.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_gid(&self, gid: Gid) -> Result<()> {
        self.0.set_gid(gid)
    }

    /// Sets the real, effective group IDs as `rgid`, `egid` respectively.
    ///
    /// If `rgid` or `egid` is `None`, the corresponding group ID will leave unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_regid(&self, rgid: Option<Gid>, egid: Option<Gid>) -> Result<()> {
        self.0.set_regid(rgid, egid)
    }

    /// Sets the real, effective, saved-set group IDs as `rgid`, `egid`, `sgid` respectively.
    ///
    /// If `rgid`, `egid`, or `sgid` is `None`, the corresponding group ID will leave unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_resgid(
        &self,
        rgid: Option<Gid>,
        egid: Option<Gid>,
        sgid: Option<Gid>,
    ) -> Result<()> {
        self.0.set_resgid(rgid, egid, sgid)
    }

    /// Sets the filesystem group ID as `fsgid` and returns the original filesystem group ID.
    ///
    /// If `fsgid` is `None`, leaves the filesystem group ID unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_fsgid(&self, fsgid: Option<Gid>) -> core::result::Result<Gid, Gid> {
        self.0.set_fsgid(fsgid)
    }

    /// Sets the effective group ID as `egid`.
    ///
    /// This method should only be used when executing a file whose `setgid` bit is set.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_egid(&self, egid: Gid) {
        self.0.set_egid(egid);
    }

    /// Sets the saved-set group ID as the same of the effective group ID.
    ///
    /// This method should only be used when executing a new executable file.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn reset_sgid(&self) {
        let egid = self.0.egid();
        self.0.set_sgid(egid);
    }

    // *********** Supplementary Groups methods **********

    /// Acquires the read lock of supplementary group IDs.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn groups(&self) -> RwLockReadGuard<'_, BTreeSet<Gid>, PreemptDisabled> {
        self.0.groups()
    }

    /// Acquires the write lock of supplementary group IDs.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn groups_mut(&self) -> RwLockWriteGuard<'_, BTreeSet<Gid>, PreemptDisabled> {
        self.0.groups_mut()
    }

    // *********** Linux Capabilities methods **********

    /// Gets the capabilities that child processes can inherit.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn inheritable_capset(&self) -> CapSet {
        self.0.inheritable_capset()
    }

    /// Gets the capabilities that are a process can potentially be granted.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn permitted_capset(&self) -> CapSet {
        self.0.permitted_capset()
    }

    /// Gets the capabilities that we can actually use.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn effective_capset(&self) -> CapSet {
        self.0.effective_capset()
    }

    /// Sets the capabilities that child processes can inherit.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_inheritable_capset(&self, inheritable_capset: CapSet) {
        self.0.set_inheritable_capset(inheritable_capset);
    }

    /// Sets the capabilities that are a process can potentially be granted.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_permitted_capset(&self, permitted_capset: CapSet) {
        self.0.set_permitted_capset(permitted_capset);
    }

    /// Sets the capabilities that we can actually use.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_effective_capset(&self, effective_capset: CapSet) {
        self.0.set_effective_capset(effective_capset);
    }

    /// Gets keep capabilities flag.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn keep_capabilities(&self) -> bool {
        self.0.keep_capabilities()
    }

    /// Sets keep capabilities flag.
    ///
    /// If the [`SecureBits::KEEP_CAPS_LOCKED`] is set, this method will return an error.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_keep_capabilities(&self, keep_capabilities: bool) -> Result<()> {
        self.0.set_keep_capabilities(keep_capabilities)
    }

    // *********** Secure Bits methods **********

    /// Gets the secure bits.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn securebits(&self) -> SecureBits {
        self.0.securebits()
    }

    /// Sets the secure bits.
    ///
    /// If the caller does not have the `CAP_SETPCAP` capability, or if it tries to set locked
    /// bits, this method will return an error.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_securebits(&self, securebits: SecureBits) -> Result<()> {
        self.0.set_securebits(securebits)
    }
}
