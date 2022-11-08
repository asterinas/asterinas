use crate::process::{signal::sig_num::SigNum, Pid};

use super::Signal;

pub type Uid = usize;

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

    pub fn uid(&self) -> Uid {
        self.uid
    }

    pub fn kind(&self) -> UserSignalKind {
        self.kind
    }
}

impl Signal for UserSignal {
    fn num(&self) -> SigNum {
        self.num
    }
}
