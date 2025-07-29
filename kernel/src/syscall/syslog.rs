// SPDX-License-Identifier: MPL-2.0

//! Syslog system call implementation for Asterinas
//!
//! This module provides a clean and simple implementation of the syslog system call
//! that supports kernel log buffer operations compatible with Linux's klogctl/syslog interface.

use alloc::vec::Vec;

use ostd::sync::SpinLock;

use super::SyscallReturn;
use crate::{prelude::*, process::credentials::capabilities::CapSet};

/// Syslog action constants (compatible with Linux)
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

/// Console log level constants
const CONSOLE_LOGLEVEL_DEFAULT: i32 = 7;
const CONSOLE_LOGLEVEL_MIN: i32 = 1;
const CONSOLE_LOGLEVEL_MAX: i32 = 8;

/// dmesg_restrict setting - when true, only privileged users can read kernel messages
static DMESG_RESTRICT: SpinLock<bool> = SpinLock::new(false);

/// Main syslog syscall implementation
pub fn sys_syslog(action: i32, buf_ptr: Vaddr, len: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!(
        "syslog: action={}, buf_ptr=0x{:x}, len={}",
        action, buf_ptr, len
    );

    // Check basic parameter validity
    if len < 0 {
        return_errno_with_message!(Errno::EINVAL, "negative buffer length");
    }

    match action {
        SYSLOG_ACTION_CLOSE | SYSLOG_ACTION_OPEN => {
            // These are no-ops in our implementation
            Ok(SyscallReturn::Return(0))
        }

        SYSLOG_ACTION_READ => {
            check_syslog_permission_detailed(ctx, action)?;
            if len == 0 {
                return Ok(SyscallReturn::Return(0));
            }
            read_kernel_log(buf_ptr, len as usize, ctx, true)
        }

        SYSLOG_ACTION_READ_ALL => {
            check_syslog_permission_detailed(ctx, action)?;
            if len == 0 {
                return Ok(SyscallReturn::Return(0));
            }
            read_kernel_log(buf_ptr, len as usize, ctx, false)
        }

        SYSLOG_ACTION_READ_CLEAR => {
            check_syslog_permission_detailed(ctx, action)?;
            if len == 0 {
                aster_logger::clear_kernel_log();
                return Ok(SyscallReturn::Return(0));
            }
            let bytes_read = read_kernel_log(buf_ptr, len as usize, ctx, false)?;
            aster_logger::clear_kernel_log();
            Ok(bytes_read)
        }

        SYSLOG_ACTION_CLEAR => {
            check_syslog_permission_detailed(ctx, action)?;
            aster_logger::clear_kernel_log();
            Ok(SyscallReturn::Return(0))
        }

        SYSLOG_ACTION_CONSOLE_OFF => {
            check_syslog_permission_detailed(ctx, action)?;
            set_console_log_level(CONSOLE_LOGLEVEL_MIN)
        }

        SYSLOG_ACTION_CONSOLE_ON => {
            check_syslog_permission_detailed(ctx, action)?;
            set_console_log_level(CONSOLE_LOGLEVEL_DEFAULT)
        }

        SYSLOG_ACTION_CONSOLE_LEVEL => {
            check_syslog_permission_detailed(ctx, action)?;
            if !(CONSOLE_LOGLEVEL_MIN..=CONSOLE_LOGLEVEL_MAX).contains(&len) {
                return_errno_with_message!(Errno::EINVAL, "invalid console log level");
            }
            set_console_log_level(len)
        }

        SYSLOG_ACTION_SIZE_UNREAD => {
            check_syslog_permission_detailed(ctx, action)?;
            let size = aster_logger::get_kernel_log_unread_size();
            Ok(SyscallReturn::Return(size as isize))
        }

        SYSLOG_ACTION_SIZE_BUFFER => {
            check_syslog_permission_detailed(ctx, action)?;
            let size = aster_logger::get_kernel_log_buffer_size();
            Ok(SyscallReturn::Return(size as isize))
        }

        _ => return_errno_with_message!(Errno::EINVAL, "invalid syslog action"),
    }
}

/// Check if the current process has permission to perform syslog operations
/// with detailed permission checking based on dmesg_restrict
fn check_syslog_permission_detailed(ctx: &Context, action: i32) -> Result<()> {
    let credentials = ctx.posix_thread.credentials();
    let effective_caps = credentials.effective_capset();

    // Actions 3 (READ_ALL), 9 (SIZE_UNREAD) and 10 (SIZE_BUFFER) may allow non-privileged access
    // when dmesg_restrict is disabled
    if (action == SYSLOG_ACTION_READ_ALL
        || action == SYSLOG_ACTION_SIZE_UNREAD
        || action == SYSLOG_ACTION_SIZE_BUFFER)
        && !*DMESG_RESTRICT.lock()
    {
        return Ok(());
    }

    // For other operations or when dmesg_restrict is enabled, require privileges
    if effective_caps.contains(CapSet::SYSLOG) || effective_caps.contains(CapSet::SYS_ADMIN) {
        Ok(())
    } else {
        return_errno_with_message!(Errno::EPERM, "operation not permitted");
    }
}

/// Read data from kernel log buffer
fn read_kernel_log(
    buf_ptr: Vaddr,
    len: usize,
    ctx: &Context,
    destructive: bool,
) -> Result<SyscallReturn> {
    if buf_ptr == 0 {
        return_errno_with_message!(Errno::EFAULT, "null buffer pointer");
    }

    let mut temp_buf = vec![0u8; len];
    let bytes_read = if destructive {
        aster_logger::read_kernel_log_destructive(&mut temp_buf)
    } else {
        aster_logger::read_kernel_log_all(&mut temp_buf)
    };

    if bytes_read > 0 {
        // Copy data to user space
        let user_space = ctx.user_space();
        for (i, &byte) in temp_buf.iter().take(bytes_read).enumerate() {
            user_space.write_val(buf_ptr + i, &byte)?;
        }
    }

    Ok(SyscallReturn::Return(bytes_read as isize))
}

/// Set console log level
fn set_console_log_level(level: i32) -> Result<SyscallReturn> {
    aster_logger::set_console_log_level(level);
    Ok(SyscallReturn::Return(0))
}
