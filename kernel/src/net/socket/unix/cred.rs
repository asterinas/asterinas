// SPDX-License-Identifier: MPL-2.0

use aster_rights::{Dup, Read, ReadDupOp, ReadOp, TRights};
use aster_rights_proc::require;

use crate::{
    prelude::*,
    process::{posix_thread::AsPosixThread, Credentials, Gid, Pid, Uid},
};

pub struct SocketCred<R = ReadOp> {
    pid: Pid,
    cred: Credentials<R>,
}

impl SocketCred<ReadOp> {
    pub fn new() -> Self {
        let pid = current!().pid();
        let cred = current_thread!().as_posix_thread().unwrap().credentials();

        Self { pid, cred }
    }
}

impl SocketCred<ReadDupOp> {
    pub fn new() -> Self {
        let pid = current!().pid();
        let cred = current_thread!()
            .as_posix_thread()
            .unwrap()
            .credentials_dup();

        Self { pid, cred }
    }
}

impl<R: TRights> SocketCred<R> {
    #[require(R > Read)]
    pub fn to_c_user_cred(&self) -> CUserCred {
        CUserCred {
            pid: self.pid,
            uid: self.cred.euid(),
            gid: self.cred.egid(),
        }
    }

    #[require(R > Read)]
    pub fn groups(&self) -> Arc<[Gid]> {
        self.cred.groups().iter().cloned().collect()
    }

    #[require(R > R1)]
    pub fn restrict<R1: TRights>(self) -> SocketCred<R1> {
        let Self { pid, cred } = self;
        SocketCred {
            pid,
            cred: cred.restrict(),
        }
    }

    #[require(R > Dup)]
    pub fn dup(&self) -> Self {
        Self {
            pid: self.pid,
            cred: self.cred.dup(),
        }
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct CUserCred {
    pid: Pid,
    uid: Uid,
    gid: Gid,
}

impl CUserCred {
    pub(in crate::net) const fn new_unknown() -> Self {
        Self {
            pid: 0,
            uid: Uid::new(u32::MAX),
            gid: Gid::new(u32::MAX),
        }
    }
}
