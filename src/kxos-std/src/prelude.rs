#![allow(unused)]

pub(crate) use alloc::boxed::Box;
pub(crate) use alloc::collections::BTreeMap;
pub(crate) use alloc::collections::VecDeque;
pub(crate) use alloc::ffi::CString;
pub(crate) use alloc::sync::Arc;
pub(crate) use alloc::sync::Weak;
#[allow(unused)]
pub(crate) use alloc::vec;
pub(crate) use alloc::vec::Vec;
pub(crate) use bitflags::bitflags;
pub(crate) use core::ffi::CStr;
pub(crate) use kxos_frame::config::PAGE_SIZE;
pub(crate) use kxos_frame::vm::Vaddr;
#[allow(unused)]
pub(crate) use kxos_frame::{debug, error, info, trace, warn};
pub(crate) use spin::Mutex;

#[macro_export]
macro_rules! current {
    () => {
        crate::process::Process::current()
    };
}

pub(crate) use crate::current;
pub(crate) use crate::error::{Errno, Error};
pub(crate) use lazy_static::lazy_static;
pub(crate) type Result<T> = core::result::Result<T, Error>;
pub(crate) use crate::{return_errno, return_errno_with_message};
