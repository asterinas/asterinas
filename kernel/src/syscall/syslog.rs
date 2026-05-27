// SPDX-License-Identifier: MPL-2.0

use aster_logger::{self, LinuxConsoleLogLevel, LOG_BUFFER_CAPACITY};
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

/// Temporary buffer size for copying data between kernel and user space.
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
            if aster_logger::klog().dmesg_restrict() {
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
            aster_logger::klog().mark_clear();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleOff => {
            ensure_cap(ctx)?;
            aster_logger::klog().disable_console();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleOn => {
            ensure_cap(ctx)?;
            aster_logger::klog().restore_console_level();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleLevel => {
            ensure_cap(ctx)?;
            let raw = len as i32;
            if !(1..=8).contains(&raw) {
                return_errno_with_message!(Errno::EINVAL, "console level out of range");
            }
            let new_level = LinuxConsoleLogLevel::from_raw(raw)
                .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid console level"))?;
            aster_logger::klog().set_console_level(new_level);
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::SizeUnread => {
            if aster_logger::klog().dmesg_restrict() {
                ensure_cap(ctx)?;
            }
            Ok(SyscallReturn::Return(
                aster_logger::klog().size_unread() as isize
            ))
        }
        SysLogAction::SizeBuffer => {
            if aster_logger::klog().dmesg_restrict() {
                ensure_cap(ctx)?;
            }
            Ok(SyscallReturn::Return(LOG_BUFFER_CAPACITY as isize))
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
            aster_logger::klog().wait_nonempty();
        }
        let to_take = core::cmp::min(len - copied, tmp.len());
        let n = aster_logger::klog().read(&mut tmp[..to_take]);
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
    let klog = aster_logger::klog();
    let at_tail = klog.snapshot_tail();

    while copied < len {
        let to_take = core::cmp::min(len - copied, tmp.len());
        let n = klog.read_all(&mut tmp[..to_take], offset, len, at_tail);
        if n == 0 {
            break;
        }
        let mut reader = VmReader::from(&tmp[..n]);
        writer.write(&mut reader)?;
        copied += n;
        offset += n;
    }

    if clear_after {
        klog.mark_clear();
    }

    Ok(copied)
}
