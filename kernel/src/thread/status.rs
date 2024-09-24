// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicU8;

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use int_to_c_enum::TryFromInt;

define_atomic_version_of_integer_like_type!(ThreadStatus, try_from = true, {
    #[derive(Debug)]
    pub struct AtomicThreadStatus(AtomicU8);
});

#[derive(Clone, Copy, PartialEq, Eq, Debug, TryFromInt)]
#[repr(u8)]
pub enum ThreadStatus {
    Init = 0,
    Running = 1,
    Exited = 2,
    Stopped = 3,
}

impl ThreadStatus {
    pub fn is_running(&self) -> bool {
        *self == ThreadStatus::Running
    }

    pub fn is_exited(&self) -> bool {
        *self == ThreadStatus::Exited
    }

    pub fn is_stopped(&self) -> bool {
        *self == ThreadStatus::Stopped
    }
}

impl From<ThreadStatus> for u8 {
    fn from(value: ThreadStatus) -> Self {
        value as u8
    }
}
