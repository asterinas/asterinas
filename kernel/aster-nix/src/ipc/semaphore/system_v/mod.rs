// SPDX-License-Identifier: MPL-2.0

//! System V semaphore.

use bitflags::bitflags;

pub mod sem;
pub mod sem_set;

bitflags! {
    pub struct PermissionMode: u16{
        const ALTER  = 0o002;
        const WRITE  = 0o002;
        const READ   = 0o004;
    }
}

pub(super) fn init() {
    sem_set::init();
}
