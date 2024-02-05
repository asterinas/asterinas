// SPDX-License-Identifier: MPL-2.0

//! The process status

use super::TermStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    // Not ready to run
    Uninit,
    /// Can be scheduled to run
    Runnable,
    /// Exit while not reaped by parent
    Zombie(TermStatus),
}

impl ProcessStatus {
    pub fn set_zombie(&mut self, term_status: TermStatus) {
        *self = ProcessStatus::Zombie(term_status);
    }

    pub fn is_zombie(&self) -> bool {
        matches!(self, ProcessStatus::Zombie(_))
    }

    pub fn set_runnable(&mut self) {
        *self = ProcessStatus::Runnable;
    }

    pub fn is_runnable(&self) -> bool {
        *self == ProcessStatus::Runnable
    }
}
