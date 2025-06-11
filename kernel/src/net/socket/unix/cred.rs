// SPDX-License-Identifier: MPL-2.0

use aster_rights::ReadOp;

use crate::{
    prelude::*,
    process::{posix_thread::AsPosixThread, Credentials, Gid, Pid, Uid},
};

#[derive(Debug, Clone)]
pub(super) struct SocketCred {
    cred: CUserCred,
    groups: Arc<[Gid]>,
}

impl SocketCred {
    pub(super) fn new_current() -> Self {
        let credentials = current_thread!().as_posix_thread().unwrap().credentials();

        let cred = CUserCred::new_current(&credentials);
        let groups = credentials.groups().iter().cloned().collect();

        Self { cred, groups }
    }

    pub(super) fn cred(&self) -> &CUserCred {
        &self.cred
    }

    pub(super) fn groups(&self) -> &Arc<[Gid]> {
        &self.groups
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
    fn new_current(credentials: &Credentials<ReadOp>) -> Self {
        let pid = current!().pid();

        Self {
            pid,
            uid: credentials.euid(),
            gid: credentials.egid(),
        }
    }
}
