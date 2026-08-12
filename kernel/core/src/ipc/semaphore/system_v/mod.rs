// SPDX-License-Identifier: MPL-2.0

//! System V semaphore.

use bitflags::bitflags;

pub(crate) mod sem;
pub(crate) mod sem_set;

bitflags! {
    pub(crate) struct PermissionMode: u16{
        const ALTER  = 0o002;
        const WRITE  = 0o002;
        const READ   = 0o004;
    }
}
