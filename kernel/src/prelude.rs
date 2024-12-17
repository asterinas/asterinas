// SPDX-License-Identifier: MPL-2.0

#![allow(unused)]

pub(crate) use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet, LinkedList, VecDeque},
    ffi::CString,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
pub(crate) use core::{any::Any, ffi::CStr, fmt::Debug};

pub(crate) use bitflags::bitflags;
pub(crate) use int_to_c_enum::TryFromInt;
pub(crate) use log::{debug, error, info, log_enabled, trace, warn};
pub(crate) use ostd::{
    mm::{FallibleVmRead, FallibleVmWrite, Vaddr, VmReader, VmWriter, PAGE_SIZE},
    sync::{Mutex, MutexGuard, RwLock, RwMutex, SpinLock, SpinLockGuard},
    Pod,
};

/// return current process
#[macro_export]
macro_rules! current {
    () => {
        $crate::process::Process::current().unwrap()
    };
}

/// Returns the current thread.
///
/// # Panics
///
/// This macro will panic if the current task is not associated with a thread.
///
/// Except for unit tests, all tasks should be associated with threads. To write code that can be
/// called directly in unit tests, consider using [`Thread::current`] instead.
///
/// [`Thread::current`]: crate::thread::Thread::current
#[macro_export]
macro_rules! current_thread {
    () => {
        $crate::thread::Thread::current().expect("the current task is not associated with a thread")
    };
}

pub(crate) use aster_logger::{print, println};

pub(crate) use crate::{
    context::{Context, CurrentUserSpace, ReadCString},
    current, current_thread,
    error::{Errno, Error},
    process::signal::Pause,
    time::{wait::WaitTimeout, Clock},
};
pub(crate) type Result<T> = core::result::Result<T, Error>;
pub(crate) use crate::{return_errno, return_errno_with_message};
