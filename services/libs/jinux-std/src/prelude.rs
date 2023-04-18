#![allow(unused)]

pub(crate) use alloc::boxed::Box;
pub(crate) use alloc::collections::BTreeMap;
pub(crate) use alloc::collections::BTreeSet;
pub(crate) use alloc::collections::LinkedList;
pub(crate) use alloc::collections::VecDeque;
pub(crate) use alloc::ffi::CString;
pub(crate) use alloc::string::String;
pub(crate) use alloc::string::ToString;
pub(crate) use alloc::sync::Arc;
pub(crate) use alloc::sync::Weak;
pub(crate) use alloc::vec;
pub(crate) use alloc::vec::Vec;
pub(crate) use bitflags::bitflags;
pub(crate) use core::any::Any;
pub(crate) use core::ffi::CStr;
pub(crate) use jinux_frame::config::PAGE_SIZE;
pub(crate) use jinux_frame::sync::{Mutex, MutexGuard};
pub(crate) use jinux_frame::vm::Vaddr;
pub(crate) use jinux_frame::{print, println};
pub(crate) use log::{debug, error, info, trace, warn};
pub(crate) use pod::Pod;
pub(crate) use spin::RwLock;

/// return current process
#[macro_export]
macro_rules! current {
    () => {
        crate::process::Process::current()
    };
}

/// return current thread
#[macro_export]
macro_rules! current_thread {
    () => {
        crate::thread::Thread::current()
    };
}

pub(crate) use crate::current;
pub(crate) use crate::current_thread;
pub(crate) use crate::error::{Errno, Error};
pub(crate) use lazy_static::lazy_static;
pub(crate) type Result<T> = core::result::Result<T, Error>;
pub(crate) use crate::{return_errno, return_errno_with_message};
