// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use aster_rights::{Dup, Read, TRights, Write};
use aster_rights_proc::require;
use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard};

use super::{capabilities::CapSet, credentials_::Credentials_, Credentials, Gid, Uid};
use crate::prelude::*;

impl<R: TRights> Credentials<R> {
    /// Creates a root `Credentials`. This method can only be used when creating the first process
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

    // *********** Uid methods **********

    /// Gets real user id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn ruid(&self) -> Uid {
        self.0.ruid()
    }

    /// Gets effective user id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn euid(&self) -> Uid {
        self.0.euid()
    }

    /// Gets saved-set user id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn suid(&self) -> Uid {
        self.0.suid()
    }

    /// Gets file system user id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn fsuid(&self) -> Uid {
        self.0.fsuid()
    }

    /// Sets uid. If self is privileged, sets the effective, real, saved-set user ids as `uid`,
    /// Otherwise, sets effective user id as `uid`.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_uid(&self, uid: Uid) {
        self.0.set_uid(uid);
    }

    /// Sets real, effective user ids as `ruid`, `euid` respectively. if `ruid` or `euid`
    /// is `None`, the corresponding user id will leave unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_reuid(&self, ruid: Option<Uid>, euid: Option<Uid>) -> Result<()> {
        self.0.set_reuid(ruid, euid)
    }

    /// Sets real, effective, saved-set user ids as `ruid`, `euid`, `suid` respectively. if
    /// `ruid`, `euid` or `suid` is `None`, the corresponding user id will leave unchanged.
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

    /// Sets file system user id as `fsuid`. Returns the original file system user id.
    /// If `fsuid` is None, leaves file system user id unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_fsuid(&self, fsuid: Option<Uid>) -> Result<Uid> {
        self.0.set_fsuid(fsuid)
    }

    /// Sets effective user id as `euid`. This method should only be used when executing a file
    /// whose `setuid` bit is set.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_euid(&self, euid: Uid) {
        self.0.set_euid(euid);
    }

    /// Sets saved-set user id as the same of effective user id. This method should only be used when
    /// executing a new executable file.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn reset_suid(&self) {
        let euid = self.0.euid();
        self.0.set_suid(euid);
    }

    // *********** Gid methods **********

    /// Gets real group id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn rgid(&self) -> Gid {
        self.0.rgid()
    }

    /// Gets effective group id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn egid(&self) -> Gid {
        self.0.egid()
    }

    /// Gets saved-set group id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn sgid(&self) -> Gid {
        self.0.sgid()
    }

    /// Gets file system group id.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn fsgid(&self) -> Gid {
        self.0.fsgid()
    }

    /// Sets gid. If self is privileged, sets the effective, real, saved-set group ids as `gid`,
    /// Otherwise, sets effective group id as `gid`.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_gid(&self, gid: Gid) {
        self.0.set_gid(gid);
    }

    /// Sets real, effective group ids as `rgid`, `egid` respectively. if `rgid` or `egid`
    /// is `None`, the corresponding group id will leave unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_regid(&self, rgid: Option<Gid>, egid: Option<Gid>) -> Result<()> {
        self.0.set_regid(rgid, egid)
    }

    /// Sets real, effective, saved-set group ids as `rgid`, `egid`, `sgid` respectively. if
    /// `rgid`, `egid` or `sgid` is `None`, the corresponding group id will leave unchanged.
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

    /// Sets file system group id as `fsgid`. Returns the original file system group id.
    /// If `fsgid` is None, leaves file system group id unchanged.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_fsgid(&self, fsgid: Option<Gid>) -> Result<Gid> {
        self.0.set_fsgid(fsgid)
    }

    /// Sets effective group id as `egid`. This method should only be used when executing a file
    /// whose `setgid` bit is set.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_egid(&self, egid: Gid) {
        self.0.set_egid(egid);
    }

    /// Sets saved-set group id as the same of effective group id. This method should only be used when
    /// executing a new executable file.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn reset_sgid(&self) {
        let egid = self.0.egid();
        self.0.set_sgid(egid);
    }

    // *********** Supplementary group methods **********

    /// Acquires the read lock of supplementary group ids.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn groups(&self) -> RwLockReadGuard<BTreeSet<Gid>, PreemptDisabled> {
        self.0.groups()
    }

    /// Acquires the write lock of supplementary group ids.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn groups_mut(&self) -> RwLockWriteGuard<BTreeSet<Gid>, PreemptDisabled> {
        self.0.groups_mut()
    }

    // *********** Linux Capability methods **********

    /// Gets the capabilities that child process can inherit.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn inheritable_capset(&self) -> CapSet {
        self.0.inheritable_capset()
    }

    /// Gets the capabilities that are permitted.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn permitted_capset(&self) -> CapSet {
        self.0.permitted_capset()
    }

    /// Gets the capabilities that actually use.
    ///
    /// This method requires the `Read` right.
    #[require(R > Read)]
    pub fn effective_capset(&self) -> CapSet {
        self.0.effective_capset()
    }

    /// Sets the capabilities that child process can inherit.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_inheritable_capset(&self, inheritable_capset: CapSet) {
        self.0.set_inheritable_capset(inheritable_capset);
    }

    /// Sets the capabilities that are permitted.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_permitted_capset(&self, permitted_capset: CapSet) {
        self.0.set_permitted_capset(permitted_capset);
    }

    /// Sets the capabilities that actually use.
    ///
    /// This method requires the `Write` right.
    #[require(R > Write)]
    pub fn set_effective_capset(&self, effective_capset: CapSet) {
        self.0.set_effective_capset(effective_capset);
    }
}
