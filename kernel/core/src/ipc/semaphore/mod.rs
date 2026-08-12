// SPDX-License-Identifier: MPL-2.0

//! Semaphore for the system, including System V semaphore and
//! POSIX semaphore.

pub(crate) mod posix;
pub(crate) mod system_v;
