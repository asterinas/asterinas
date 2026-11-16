// SPDX-License-Identifier: MPL-2.0

//! Semaphore for the system, including System V message queue and
//! POSIX message queue.

pub(crate) mod posix;
pub(crate) mod system_v;
