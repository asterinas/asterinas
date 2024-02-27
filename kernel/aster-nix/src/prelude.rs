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

pub(crate) use aster_frame::{
    config::PAGE_SIZE,
    sync::{Mutex, MutexGuard, RwLock, RwMutex, SpinLock, SpinLockGuard},
    vm::Vaddr,
};
pub(crate) use bitflags::bitflags;
pub(crate) use int_to_c_enum::TryFromInt;
pub(crate) use log::{debug, error, info, trace, warn};
pub(crate) use pod::Pod;

/// return current process
#[macro_export]
macro_rules! current {
    () => {
        $crate::process::current()
    };
}

/// return current thread
#[macro_export]
macro_rules! current_thread {
    () => {
        $crate::thread::Thread::current()
    };
}

pub(crate) use lazy_static::lazy_static;

pub(crate) use crate::{
    current, current_thread,
    error::{Errno, Error},
    print, println,
};
pub(crate) type Result<T> = core::result::Result<T, Error>;
pub(crate) use crate::{return_errno, return_errno_with_message};
