// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use super::PosixThread;
use crate::thread::{AsThread, Thread};

/// A trait to provide the `as_posix_thread` method for tasks and threads.
pub trait AsPosixThread {
    /// Returns the associated [`PosixThread`].
    fn as_posix_thread(&self) -> Option<&PosixThread>;
}

impl AsPosixThread for Thread {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.data().downcast_ref::<PosixThread>()
    }
}

impl AsPosixThread for Task {
    fn as_posix_thread(&self) -> Option<&PosixThread> {
        self.as_thread()?.as_posix_thread()
    }
}
