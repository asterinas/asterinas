// SPDX-License-Identifier: MPL-2.0

use super::signal::sig_num::SigNum;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermStatus {
    Exited(u8),
    Killed(SigNum),
}

impl TermStatus {
    /// Return as a 32-bit integer encoded as specified in wait(2) man page.
    pub fn as_u32(&self) -> u32 {
        match self {
            TermStatus::Exited(status) => (*status as u32) << 8,
            TermStatus::Killed(signum) => signum.as_u8() as u32,
        }
    }
}
