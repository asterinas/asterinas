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
pub(crate) use lazy_static::lazy_static;
pub(crate) use log::{debug, error, info, log_enabled, trace, warn};
pub(crate) use ostd::{
    mm::{Vaddr, VmReader, VmWriter, PAGE_SIZE},
    sync::{Mutex, MutexGuard, RwLock, RwMutex, SpinLock, SpinLockGuard},
    Pod,
};

pub(crate) use crate::{
    current,
    error::{Errno, Error},
    print, println,
    time::{wait::WaitTimeout, Clock},
};
pub(crate) type Result<T> = core::result::Result<T, Error>;
pub(crate) use crate::{return_errno, return_errno_with_message};
