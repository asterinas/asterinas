// SPDX-License-Identifier: MPL-2.0

#![expect(unused)]

pub use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet, LinkedList, VecDeque},
    ffi::CString,
    string::{String, ToString},
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
pub use core::{any::Any, ffi::CStr, fmt::Debug};

pub use aster_logger::{print, println};
pub use bitflags::bitflags;
pub use int_to_c_enum::TryFromInt;
pub use ostd::{
    alert, crit, debug, emerg, error, info,
    mm::{FallibleVmRead, FallibleVmWrite, PAGE_SIZE, Vaddr, VmReader, VmWriter},
    notice,
    sync::{Mutex, MutexGuard, RwLock, RwMutex, SpinLock, SpinLockGuard},
    warn,
};
pub use ostd_pod::{FromBytes, FromZeros, IntoBytes, Pod};

pub use crate::{
    context::Context,
    error::{Errno, Error},
    process::{posix_thread::AsThreadLocal, signal::Pause},
    time::{Clock, wait::WaitTimeout},
    util::ReadCString,
};
pub(crate) use crate::{
    context::{CurrentUserSpace, current, current_thread},
    error::{return_errno, return_errno_with_message},
};

pub type Result<T> = core::result::Result<T, Error>;
