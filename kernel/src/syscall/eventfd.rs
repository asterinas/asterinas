// SPDX-License-Identifier: MPL-2.0

//! `eventfd()` creates an "eventfd object" (we name it as `EventFile`)
//! which serves as a mechanism for event wait/notify.
//!
//! `EventFile` holds a `u64` integer counter.
//! Writing to `EventFile` increments the counter by the written value.
//! Reading from `EventFile` returns the current counter value and resets it
//! (It is also possible to only read 1,
//! depending on whether the `EFD_SEMAPHORE` flag is set).
//! The read/write operations may be blocked based on file flags.
//!
//! For more detailed information about this syscall,
//! refer to the man 2 eventfd documentation.
//!
//! File-descriptor operations are implemented by [`EventFile`], while the
//! shared counter and kernel-facing operations are owned by its
//! [`KernelEventFile`] component.

use super::SyscallReturn;
use crate::{
    events::{EventFile, EventFileFlags},
    fs::file::file_table::FdFlags,
    prelude::*,
};

pub fn sys_eventfd(init_val: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("init_val = 0x{:x}", init_val);

    do_sys_eventfd2(init_val, EventFileFlags::empty(), ctx)
}

pub fn sys_eventfd2(init_val: u32, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw flags = {}", flags);
    let flags = EventFileFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;
    debug!("init_val = 0x{:x}, flags = {:?}", init_val, flags);

    do_sys_eventfd2(init_val, flags, ctx)
}

fn do_sys_eventfd2(init_val: u32, flags: EventFileFlags, ctx: &Context) -> Result<SyscallReturn> {
    let event_file = EventFile::new(init_val as u64, flags);
    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd_flags = if flags.contains(EventFileFlags::EFD_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let fd = file_table_locked.insert(Arc::new(event_file), fd_flags);
    Ok(SyscallReturn::Return(fd.into()))
}
