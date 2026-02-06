// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use super::Signal;
use crate::{
    context::Context,
    process::{
        Pid, Uid,
        signal::{
            c_types::siginfo_t,
            constants::{SI_QUEUE, SI_TKILL, SI_USER},
            sig_num::SigNum,
        },
    },
};

#[derive(Debug, Clone, Copy)]
pub struct UserSignal {
    num: SigNum,
    pid: Pid,
    uid: Uid,
    kind: UserSignalKind,
}

#[derive(Debug, Copy, Clone)]
pub enum UserSignalKind {
    Kill,
    Tkill,
    Sigqueue,
}

impl UserSignal {
    pub fn new(num: SigNum, kind: UserSignalKind, pid: Pid, uid: Uid) -> Self {
        Self {
            num,
            kind,
            pid,
            uid,
        }
    }

    pub fn new_kill(num: SigNum, ctx: &Context) -> Self {
        Self {
            num,
            kind: UserSignalKind::Kill,
            pid: ctx.process.pid(),
            uid: ctx.posix_thread.credentials().ruid(),
        }
    }

    pub fn pid(&self) -> Pid {
        self.pid
    }

    pub fn kind(&self) -> UserSignalKind {
        self.kind
    }
}

impl Signal for UserSignal {
    fn num(&self) -> SigNum {
        self.num
    }

    fn to_info(&self) -> siginfo_t {
        let code = match self.kind {
            UserSignalKind::Kill => SI_USER,
            UserSignalKind::Tkill => SI_TKILL,
            UserSignalKind::Sigqueue => SI_QUEUE,
        };

        let mut info = siginfo_t::new(self.num, code);
        info.set_pid_uid(self.pid, self.uid);

        info
    }
}
