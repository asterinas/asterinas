// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use ostd::task::{CurrentTask, Task};

use super::{CurrentPosixThread, PosixThread};
use crate::thread::{AsThread, CurrentThread, CurrentThreadRef, Thread};

/// A trait to provide the `as_posix_thread` method for tasks and threads.
pub trait AsPosixThread {
    /// Returns the associated [`PosixThread`].
    fn as_posix_thread(&self) -> Option<&PosixThread>;

    /// Returns the associated [`CurrentPosixThread`]
    /// if `self` is the current task or current thread.
    fn as_current_posix_thread(&self) -> Option<CurrentPosixThread>;
}

impl AsPosixThread for Thread {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.data().downcast_ref::<PosixThread>()
    }

    fn as_current_posix_thread(&self) -> Option<CurrentPosixThread> {
        None
    }
}

impl AsPosixThread for CurrentThread {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.deref().as_posix_thread()
    }

    fn as_current_posix_thread(&self) -> Option<CurrentPosixThread> {
        self.as_posix_thread().map(CurrentPosixThread)
    }
}

impl AsPosixThread for CurrentThreadRef<'_> {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.deref().as_posix_thread()
    }

    fn as_current_posix_thread(&self) -> Option<CurrentPosixThread> {
        self.as_posix_thread().map(CurrentPosixThread)
    }
}

impl AsPosixThread for Task {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.as_thread()?.as_posix_thread()
    }

    fn as_current_posix_thread(&self) -> Option<CurrentPosixThread> {
        None
    }
}

impl AsPosixThread for CurrentTask {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.deref().as_posix_thread()
    }

    fn as_current_posix_thread(&self) -> Option<CurrentPosixThread> {
        self.as_thread()?.as_posix_thread().map(CurrentPosixThread)
    }
}
