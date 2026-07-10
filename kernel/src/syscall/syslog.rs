// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroUsize;

use aster_logger::{KernelLog, LinuxConsoleLogLevel};
use int_to_c_enum::TryFromInt;

use super::SyscallReturn;
use crate::{prelude::*, process::credentials::capabilities::CapSet};

/// Actions for the syslog system call.
///
/// These actions control how the kernel log buffer is accessed and managed.
/// See `man 2 syslog` for detailed documentation.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
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
    /// Disable printing of messages to the console.
    ConsoleOff = 6,
    /// Enable printing of messages to the console.
    ConsoleOn = 7,
    /// Set the console log level.
    ConsoleLevel = 8,
    /// Return the number of unread characters in the log buffer.
    SizeUnread = 9,
    /// Return the size of the log buffer.
    SizeBuffer = 10,
}

pub fn sys_syslog(action: i32, buf: Vaddr, len: i32, ctx: &Context) -> Result<SyscallReturn> {
    let klog = aster_logger::klog();

    if action_requires_cap(action, klog) {
        check_cap(ctx)?;
    }

    let action = SysLogAction::try_from(action)?;

    match action {
        SysLogAction::Close | SysLogAction::Open => Ok(SyscallReturn::Return(0)),
        SysLogAction::Read => {
            let len = validate_read_args(buf, len)?;
            let Some(len) = NonZeroUsize::new(len) else {
                return Ok(SyscallReturn::Return(0));
            };

            Ok(SyscallReturn::Return(
                read_destructive(buf, len, ctx, klog)? as isize,
            ))
        }
        SysLogAction::ReadAll => {
            let len = validate_read_args(buf, len)?;
            let Some(len) = NonZeroUsize::new(len) else {
                return Ok(SyscallReturn::Return(0));
            };

            Ok(SyscallReturn::Return(
                pick_latest(buf, len, ctx, klog, false)? as isize,
            ))
        }
        SysLogAction::ReadClear => {
            let len = validate_read_args(buf, len)?;
            let Some(len) = NonZeroUsize::new(len) else {
                return Ok(SyscallReturn::Return(0));
            };

            Ok(SyscallReturn::Return(
                pick_latest(buf, len, ctx, klog, true)? as isize,
            ))
        }
        SysLogAction::Clear => {
            klog.mark_clear();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleOff => {
            klog.disable_console();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleOn => {
            klog.enable_console();
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::ConsoleLevel => {
            let new_level = if let Ok(raw_level) = u8::try_from(len)
                && let Ok(level) = LinuxConsoleLogLevel::try_from(raw_level)
            {
                level
            } else {
                return_errno_with_message!(Errno::EINVAL, "the console level is out of range");
            };

            klog.set_console_level(new_level);
            Ok(SyscallReturn::Return(0))
        }
        SysLogAction::SizeUnread => Ok(SyscallReturn::Return(klog.size_unread() as isize)),
        SysLogAction::SizeBuffer => Ok(SyscallReturn::Return(klog.capacity() as isize)),
    }
}

/// Returns whether the raw syslog action requires privilege.
///
/// When `dmesg_restrict` is enabled, every syslog action requires
/// `CAP_SYS_ADMIN` or `CAP_SYSLOG`. Otherwise, Linux allows
/// `SYSLOG_ACTION_READ_ALL` and `SYSLOG_ACTION_SIZE_BUFFER` without these
/// capabilities, while all other actions, including unknown action numbers,
/// remain privileged.
fn action_requires_cap(action: i32, klog: &KernelLog) -> bool {
    if klog.dmesg_restrict() {
        return true;
    }

    !matches!(
        SysLogAction::try_from(action),
        Ok(SysLogAction::ReadAll | SysLogAction::SizeBuffer)
    )
}

/// Checks whether the current thread may perform privileged syslog actions.
fn check_cap(ctx: &Context) -> Result<()> {
    let credentials = ctx.posix_thread.credentials();
    if !credentials
        .effective_capset()
        .intersects(CapSet::SYS_ADMIN | CapSet::SYSLOG)
    {
        return_errno_with_message!(
            Errno::EPERM,
            "the thread does not have the required capability"
        );
    }
    Ok(())
}

fn validate_read_args(buf: Vaddr, len: i32) -> Result<usize> {
    if buf == 0 || len < 0 {
        return_errno_with_message!(Errno::EINVAL, "syslog read arguments are invalid");
    }

    Ok(len as usize)
}

/// Reads and removes kernel log bytes for `SYSLOG_ACTION_READ`.
///
/// This action blocks until at least one byte is available, then drains up to
/// `len` bytes into the user buffer.
fn read_destructive(
    buf: Vaddr,
    len: NonZeroUsize,
    ctx: &Context,
    klog: &KernelLog,
) -> Result<usize> {
    let user_space = ctx.user_space();
    let mut writer = user_space.writer(buf, len.get())?;

    Ok(klog
        .wait_queue()
        .pause_until(|| match klog.read(&mut writer) {
            Ok(0) => None,
            res => Some(res),
        })
        .map_err(|err| match err.error() {
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })??)
}

/// Picks kernel log bytes without consuming them.
///
/// If `clear_after_pick` is true, atomically advances the pick head after picking.
fn pick_latest(
    buf: Vaddr,
    len: NonZeroUsize,
    ctx: &Context,
    klog: &KernelLog,
    clear_after_pick: bool,
) -> Result<usize> {
    let user_space = ctx.user_space();
    let mut writer = user_space.writer(buf, len.get())?;

    if clear_after_pick {
        Ok(klog.pick_latest_and_clear(&mut writer)?)
    } else {
        Ok(klog.pick_latest(&mut writer)?)
    }
}
