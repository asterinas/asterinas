// SPDX-License-Identifier: MPL-2.0

use aster_logger::{
    LinuxConsoleLogLevel, console_off, console_on, console_set_level, klog_capacity, klog_read,
    klog_read_all, klog_size_unread, klog_wait_nonempty, mark_clear, read_all_requires_cap,
};
use int_to_c_enum::TryFromInt;
use ostd::mm::VmReader;

use super::SyscallReturn;
use crate::{prelude::*, process::credentials::capabilities::CapSet, util::MultiWrite};

/// Actions for the syslog system call.
///
/// These actions control how the kernel log buffer is accessed and managed.
/// See `man 2 syslog` for detailed documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[repr(i32)]
enum SysLogAction {
    /// Close the log (currently a no-op).
    Close = 0,
    /// Open the log (currently a no-op).
    Open = 1,
    /// Read from the log (blocking, destructive).
    Read = 2,
    /// Read all messages remaining in the ring buffer (non-destructive).
    ReadAll = 3,
    /// Read and clear all messages remaining in the ring buffer.
    ReadClear = 4,
    /// Clear the ring buffer.
    Clear = 5,
    /// Disable printk to console.
    ConsoleOff = 6,
    /// Enable printk to console.
    ConsoleOn = 7,
    /// Set the console log level.
    ConsoleLevel = 8,
    /// Return number of unread characters in the log buffer.
    SizeUnread = 9,
    /// Return the size of the log buffer.
    SizeBuffer = 10,
}

const TMP_BUF: usize = 512;

pub fn sys_syslog(action: i32, buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let action = SysLogAction::try_from(action)
        .map_err(|_| Error::with_message(Errno::EINVAL, "unknown syslog action"))?;

    match action {
        SysLogAction::Close | SysLogAction::Open => Ok(SyscallReturn::Return(0)),
        SysLogAction::Read => {
            ensure_cap(ctx)?;
            Ok(SyscallReturn::Return(
                read_destructive(buf, len, ctx)? as isize
            ))
        }
        SysLogAction::ReadAll => {
            if read_all_requires_cap() {
                ensure_cap(ctx)?;
            }
            Ok(SyscallReturn::Return(
                read_all(buf, len, ctx, false)? as isize
            ))
        }
        SysLogAction::ReadClear => {
            ensure_cap(ctx)?;
            let copied = read_all(buf, len, ctx, true)?;
            Ok(SyscallReturn::Return(copied as isize))
        }
        SysLogAction::Clear => {
            ensure_cap(ctx)?;
            mark_clear();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleOff => {
            ensure_cap(ctx)?;
            console_off();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleOn => {
            ensure_cap(ctx)?;
            console_on();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleLevel => {
            ensure_cap(ctx)?;
            let new_level = LinuxConsoleLogLevel::from_raw(len as i32)
                .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid console level"))?;
            let _old = console_set_level(new_level);
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::SizeUnread => {
            ensure_cap(ctx)?;
            Ok(SyscallReturn::Return(klog_size_unread() as isize))
        }
        SysLogAction::SizeBuffer => {
            if read_all_requires_cap() {
                ensure_cap(ctx)?;
            }
            Ok(SyscallReturn::Return(klog_capacity() as isize))
        }
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
