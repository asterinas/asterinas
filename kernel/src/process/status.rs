// SPDX-License-Identifier: MPL-2.0

//! The process status.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::ExitCode;

/// The status of a process.
///
/// This maintains:
/// 1. Whether the process is a zombie (i.e., all its threads have exited);
/// 2. Whether a process is created by `vfork` and still shares the same address space with its parent.
/// 3. The exit code of the process.
#[derive(Debug)]
pub struct ProcessStatus {
    is_zombie: AtomicBool,
    is_vfork: AtomicBool,
    exit_code: AtomicU32,
}

impl Default for ProcessStatus {
    fn default() -> Self {
        Self {
            is_zombie: AtomicBool::new(false),
            is_vfork: AtomicBool::new(false),
            exit_code: AtomicU32::new(0),
        }
    }
}

impl ProcessStatus {
    /// Returns whether the process is a zombie process.
    pub fn is_zombie(&self) -> bool {
        // Use the `Acquire` memory order to make the exit code visible.
        self.is_zombie.load(Ordering::Acquire)
    }

    /// Sets the process to be a zombie process.
    ///
    /// This method should be called when the process completes its exit. The current thread must
    /// be the last thread in the process, so that no threads belonging to the process can run
    /// after it.
    pub(super) fn set_zombie(&self) {
        // Use the `Release` memory order to make the exit code visible.
        self.is_zombie.store(true, Ordering::Release);
    }

    /// Returns whether the process is a vfork process.
    pub fn is_vfork(&self) -> bool {
        self.is_vfork.load(Ordering::Acquire)
    }

    /// Sets the vfork status of the process to `is_vfork`.
    pub fn set_vfork_status(&self, is_vfork: bool) {
        // Use the `Release` memory order to make the exit code visible.
        self.is_vfork.store(is_vfork, Ordering::Release);
    }
}

impl ProcessStatus {
    /// Returns the exit code.
    pub fn exit_code(&self) -> ExitCode {
        self.exit_code.load(Ordering::Relaxed)
    }

    /// Sets the exit code.
    pub(super) fn set_exit_code(&self, exit_code: ExitCode) {
        self.exit_code.store(exit_code, Ordering::Relaxed);
    }
}
