// SPDX-License-Identifier: MPL-2.0

//! Semaphore for the system, including System V semaphore and
//! POSIX semaphore.

pub mod posix;
pub mod system_v;

pub(super) fn init() {
    system_v::init();
}
