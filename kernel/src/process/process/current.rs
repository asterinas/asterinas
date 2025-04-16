// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use super::{Pgid, Pid, Process, Sid};
use crate::prelude::*;

pub struct CurrentProcess(Arc<Process>);

impl CurrentProcess {
    pub(super) fn new(process: Arc<Process>) -> Self {
        CurrentProcess(process)
    }

    pub fn pid(&self) -> Pid {
        self.0.pid
    }

    pub fn parent_pid(&self) -> Pid {
        self.parent.pid()
    }

    pub fn pgid(&self) -> Pgid {
        let Some(pgrp) = self.process_group() else {
            return 0;
        };

        pgrp.pgid_in_ns(&self.pid_namespace).unwrap_or(0)
    }

    pub fn sid(&self) -> Sid {
        let Some(session) = self.session() else {
            return 0;
        };

        session.sid_in_ns(&self.pid_namespace).unwrap_or(0)
    }
}

impl Deref for CurrentProcess {
    type Target = Arc<Process>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
