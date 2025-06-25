// SPDX-License-Identifier: MPL-2.0

use aster_rights::{Dup, Read, ReadDupOp, ReadOp, TRights};
use aster_rights_proc::require;

use crate::{
    prelude::*,
    process::{posix_thread::AsPosixThread, Credentials, Gid, Pid, Uid},
};

pub(super) struct SocketCred<R = ReadOp> {
    pid: Pid,
    cred: Credentials<R>,
}

impl SocketCred<ReadOp> {
    pub(super) fn new_current() -> Self {
        let pid = current!().pid();
        let cred = current_thread!().as_posix_thread().unwrap().credentials();

        Self { pid, cred }
    }
}

impl SocketCred<ReadDupOp> {
    pub(super) fn new_current() -> Self {
        let pid = current!().pid();
        let cred = current_thread!()
            .as_posix_thread()
            .unwrap()
            .credentials_dup();

        Self { pid, cred }
    }
}

impl<R: TRights> SocketCred<R> {
    /// Converts to a [`CUserCred`] with the PID and the _effective_ UID/GID.
    #[require(R > Read)]
    pub(super) fn to_effective_c_cred(&self) -> CUserCred {
        CUserCred {
            pid: self.pid,
            uid: self.cred.euid(),
            gid: self.cred.egid(),
        }
    }

    /// Converts to a [`CUserCred`] with the PID and the _real_ UID/GID.
    #[require(R > Read)]
    pub(super) fn to_real_c_cred(&self) -> CUserCred {
        CUserCred {
            pid: self.pid,
            uid: self.cred.ruid(),
            gid: self.cred.rgid(),
        }
    }

    #[require(R > Read)]
    pub(super) fn groups(&self) -> Arc<[Gid]> {
        self.cred.groups().iter().cloned().collect()
    }

    #[require(R > R1)]
    pub(super) fn restrict<R1: TRights>(self) -> SocketCred<R1> {
        let Self { pid, cred } = self;
        SocketCred {
            pid,
            cred: cred.restrict(),
        }
    }

    #[require(R > Dup)]
    pub(super) fn dup(&self) -> Self {
        Self {
            pid: self.pid,
            cred: self.cred.dup(),
        }
    }
}

/// `struct ucred` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15/source/include/linux/socket.h#L183>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct CUserCred {
    pid: Pid,
    uid: Uid,
    gid: Gid,
}

impl CUserCred {
    pub(in crate::net) const fn new_invalid() -> Self {
        Self {
            pid: 0,
            uid: Uid::INVALID,
            gid: Gid::INVALID,
        }
    }

    pub(in crate::net) const fn new_overflow() -> Self {
        Self {
            pid: 0,
            uid: Uid::OVERFLOW,
            gid: Gid::OVERFLOW,
        }
    }
}
