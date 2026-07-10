// SPDX-License-Identifier: MPL-2.0

//! System V semaphore.

use bitflags::bitflags;

pub mod sem;
pub mod sem_set;
pub mod sem_undo;

bitflags! {
    pub struct PermissionMode: u16{
        const ALTER  = 0o002;
        const WRITE  = 0o002;
        const READ   = 0o004;
    }
}
