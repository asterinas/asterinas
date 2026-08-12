// SPDX-License-Identifier: MPL-2.0

//! `eventfd()` creates an eventfd object used for event notification.
//!
//! For more detailed information about this syscall,
//! refer to the man 2 eventfd documentation.

use super::SyscallReturn;
use crate::{
    events::{EventFile, EventFileFlags},
    fs::file::file_table::FdFlags,
    prelude::*,
};

pub(super) fn sys_eventfd(init_val: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("init_val = 0x{:x}", init_val);

    do_sys_eventfd2(init_val, EventFileFlags::empty(), ctx)
}

pub(super) fn sys_eventfd2(init_val: u32, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw flags = {}", flags);
    let flags = EventFileFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;
    debug!("init_val = 0x{:x}, flags = {:?}", init_val, flags);

    do_sys_eventfd2(init_val, flags, ctx)
}

fn do_sys_eventfd2(init_val: u32, flags: EventFileFlags, ctx: &Context) -> Result<SyscallReturn> {
    let event_file = Arc::new(EventFile::new(init_val as u64, flags));
    let fd_flags = if flags.contains(EventFileFlags::EFD_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd = file_table_locked.insert(event_file, fd_flags);
    Ok(SyscallReturn::Return(fd.into()))
}
