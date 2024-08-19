// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use super::Signal;
use crate::process::{
    signal::{
        c_types::siginfo_t,
        constants::{SI_QUEUE, SI_TKILL, SI_USER},
        sig_num::SigNum,
    },
    Pid, Uid,
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

        siginfo_t::new(self.num, code)
        // info.set_si_pid(self.pid);
        // info.set_si_uid(self.uid);
        // if let UserSignalKind::Sigqueue(val) = self.kind {
        //     info.set_si_value(val);
        // }
    }
}
