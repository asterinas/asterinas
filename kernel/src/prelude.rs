// SPDX-License-Identifier: MPL-2.0

#![expect(unused)]

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

pub(crate) use aster_logger::{print, println};
pub(crate) use bitflags::bitflags;
pub(crate) use int_to_c_enum::TryFromInt;
pub(crate) use ostd::{
    alert, crit, debug, emerg, error, info,
    mm::{FallibleVmRead, FallibleVmWrite, PAGE_SIZE, Vaddr, VmReader, VmWriter},
    notice,
    sync::{Mutex, MutexGuard, RwLock, RwMutex, SpinLock, SpinLockGuard},
    warn,
};
pub(crate) use ostd_pod::{FromBytes, FromZeros, IntoBytes, Pod};

pub(crate) use crate::{
    context::{Context, CurrentUserSpace, current, current_thread},
    error::{Errno, Error, return_errno, return_errno_with_message},
    process::{posix_thread::AsThreadLocal, signal::Pause},
    time::{Clock, wait::WaitTimeout},
    util::ReadCString,
};

pub(crate) type Result<T> = core::result::Result<T, Error>;
