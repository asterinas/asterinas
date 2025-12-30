// SPDX-License-Identifier: MPL-2.0

use aster_logger::{
    console_off, console_on, console_set_level, klog_capacity, klog_read, klog_read_all,
    klog_size_unread, klog_wait_nonempty, mark_clear, read_all_requires_cap,
};
use log::LevelFilter;
use ostd::mm::VmReader;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::credentials::capabilities::CapSet,
    util::MultiWrite,
};

const SYSLOG_ACTION_CLOSE: i32 = 0;
const SYSLOG_ACTION_OPEN: i32 = 1;
const SYSLOG_ACTION_READ: i32 = 2;
const SYSLOG_ACTION_READ_ALL: i32 = 3;
const SYSLOG_ACTION_READ_CLEAR: i32 = 4;
const SYSLOG_ACTION_CLEAR: i32 = 5;
const SYSLOG_ACTION_CONSOLE_OFF: i32 = 6;
const SYSLOG_ACTION_CONSOLE_ON: i32 = 7;
const SYSLOG_ACTION_CONSOLE_LEVEL: i32 = 8;
const SYSLOG_ACTION_SIZE_UNREAD: i32 = 9;
const SYSLOG_ACTION_SIZE_BUFFER: i32 = 10;

const TMP_BUF: usize = 512;

pub fn sys_syslog(action: i32, buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    match action {
        SYSLOG_ACTION_CLOSE | SYSLOG_ACTION_OPEN => Ok(SyscallReturn::Return(0)),
        SYSLOG_ACTION_READ => {
            ensure_cap(ctx)?;
            Ok(SyscallReturn::Return(read_destructive(buf, len, ctx)? as isize))
        }
        SYSLOG_ACTION_READ_ALL => {
            if read_all_requires_cap() {
                ensure_cap(ctx)?;
            }
            Ok(SyscallReturn::Return(read_all(buf, len, ctx, false)? as isize))
        }
        SYSLOG_ACTION_READ_CLEAR => {
            ensure_cap(ctx)?;
            let copied = read_all(buf, len, ctx, true)?;
            Ok(SyscallReturn::Return(copied as isize))
        }
        SYSLOG_ACTION_CLEAR => {
            ensure_cap(ctx)?;
            mark_clear();
            Ok(SyscallReturn::Return(0))
        }
        SYSLOG_ACTION_CONSOLE_OFF => {
            ensure_cap(ctx)?;
            console_off();
            Ok(SyscallReturn::Return(0))
        }
        SYSLOG_ACTION_CONSOLE_ON => {
            ensure_cap(ctx)?;
            console_on();
            Ok(SyscallReturn::Return(0))
        }
        SYSLOG_ACTION_CONSOLE_LEVEL => {
            ensure_cap(ctx)?;
            let Some(new_level) = level_from_raw(len as i32) else {
                return_errno_with_message!(Errno::EINVAL, "invalid console level");
            };
            let _old = console_set_level(new_level);
            Ok(SyscallReturn::Return(0))
        }
        SYSLOG_ACTION_SIZE_UNREAD => {
            ensure_cap(ctx)?;
            Ok(SyscallReturn::Return(klog_size_unread() as isize))
        }
        SYSLOG_ACTION_SIZE_BUFFER => {
            if read_all_requires_cap() {
                ensure_cap(ctx)?;
            }
            Ok(SyscallReturn::Return(klog_capacity() as isize))
        }
        _ => return_errno_with_message!(Errno::EINVAL, "unknown syslog action"),
    }
}

fn ensure_cap(ctx: &Context) -> Result<()> {
    let credentials = ctx.posix_thread.credentials();
    if credentials
        .effective_capset()
        .intersects(CapSet::SYS_ADMIN | CapSet::SYSLOG)
    {
        return Ok(());
    }
    return_errno_with_message!(Errno::EPERM, "operation not permitted")
}

fn read_destructive(buf: Vaddr, len: usize, ctx: &Context) -> Result<usize> {
    if len == 0 {
        return Ok(0);
    }
    let mut tmp = [0u8; TMP_BUF];
    let mut copied = 0;
    let user_space = ctx.user_space();
    let mut writer = user_space.writer(buf, len)?;

    while copied < len {
        // Block until at least one byte is available, then drain non-blocking.
        if copied == 0 {
            klog_wait_nonempty();
        }
        let to_take = core::cmp::min(len - copied, tmp.len());
        let n = klog_read(&mut tmp[..to_take]);
        if n == 0 {
            // If we raced with another reader after waiting, wait again for the first byte.
            if copied == 0 {
                // Retry waiting for data.
                continue;
            }
            break;
        }
        let mut reader = VmReader::from(&tmp[..n]);
        writer.write(&mut reader)?;
        copied += n;
    }
    Ok(copied)
}

fn read_all(buf: Vaddr, len: usize, ctx: &Context, clear_after: bool) -> Result<usize> {
    if len == 0 {
        return Ok(0);
    }
    let mut tmp = [0u8; TMP_BUF];
    let mut copied = 0;
    let mut offset = 0;
    let user_space = ctx.user_space();
    let mut writer = user_space.writer(buf, len)?;

    while copied < len {
        let to_take = core::cmp::min(len - copied, tmp.len());
        let n = klog_read_all(&mut tmp[..to_take], offset, len);
        if n == 0 {
            break;
        }
        let mut reader = VmReader::from(&tmp[..n]);
        writer.write(&mut reader)?;
        copied += n;
        offset += n;
    }

    if clear_after {
        mark_clear();
    }

    Ok(copied)
}

fn level_from_raw(raw: i32) -> Option<LevelFilter> {
    match raw {
        1..=4 => Some(LevelFilter::Error),
        5 => Some(LevelFilter::Warn),
        6 => Some(LevelFilter::Info),
        7 => Some(LevelFilter::Debug),
        8 => Some(LevelFilter::Trace),
        _ => None,
    }
}

fn level_to_raw(level: LevelFilter) -> u8 {
    match level {
        LevelFilter::Off => 0,
        LevelFilter::Error => 4,
        LevelFilter::Warn => 5,
        LevelFilter::Info => 6,
        LevelFilter::Debug => 7,
        LevelFilter::Trace => 8,
    }
}

