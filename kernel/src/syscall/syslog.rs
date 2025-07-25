// SPDX-License-Identifier: MPL-2.0

//! Syslog system call implementation for Asterinas
//!
//! This module provides a clean and simple implementation of the syslog system call
//! that supports kernel log buffer operations compatible with Linux's klogctl/syslog interface.

use alloc::{format, vec::Vec};

use ostd::sync::SpinLock;
use spin::Once;

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

/// Kernel log buffer configuration
const KERNEL_LOG_BUFFER_SIZE: usize = 65536; // 64KB buffer

/// Kernel log priority levels (compatible with Linux)
const KERN_ERR: i32 = 3; // Error messages
const KERN_WARNING: i32 = 4; // Warning messages
const KERN_INFO: i32 = 6; // Informational messages
const KERN_DEBUG: i32 = 7; // Debug messages

/// Efficient circular kernel log buffer with corrected logic
struct KernelLogBuffer {
    /// Fixed-size buffer for log data
    buffer: Vec<u8>,
    /// Position of the oldest data in the buffer
    start_pos: usize,
    /// Number of valid bytes in buffer (0 <= count <= buffer.len())
    count: usize,
    /// Position for destructive reads (tracks what has been read)
    read_pos: usize,
    /// Total bytes written since buffer creation (for statistics)
    total_written: usize,
}

impl KernelLogBuffer {
    /// Create a new kernel log buffer
    fn new() -> Self {
        let buffer = vec![0u8; KERNEL_LOG_BUFFER_SIZE];
        Self {
            buffer,
            start_pos: 0,
            count: 0,
            read_pos: 0,
            total_written: 0,
        }
    }

    /// Add a log message to the buffer
    fn append(&mut self, data: &[u8]) {
        // Limit message size to prevent overwhelming the buffer
        let max_msg_size = self.buffer.len() / 4; // Max 25% of buffer per message
        let data = if data.len() > max_msg_size {
            &data[..max_msg_size]
        } else {
            data
        };

        self.total_written += data.len();

        for &byte in data {
            let write_pos = (self.start_pos + self.count) % self.buffer.len();
            self.buffer[write_pos] = byte;

            if self.count < self.buffer.len() {
                // Buffer not full, just increase count
                self.count += 1;
            } else {
                // Buffer is full, advance start_pos to overwrite oldest data
                self.start_pos = (self.start_pos + 1) % self.buffer.len();

                // If read_pos was pointing to the data we just overwrote, advance it too
                if self.read_pos == self.start_pos {
                    self.read_pos = (self.read_pos + 1) % self.buffer.len();
                }
            }
        }
    }

    /// Read data from buffer (destructive read, advances read pointer)
    fn read_destructive(&mut self, buf: &mut [u8]) -> usize {
        let unread_count = self.unread_count();
        let to_read = core::cmp::min(buf.len(), unread_count);

        for (i, item) in buf.iter_mut().enumerate().take(to_read) {
            // Calculate actual position in circular buffer
            let pos = (self.read_pos + i) % self.buffer.len();
            *item = self.buffer[pos];
        }

        // Update read position
        self.read_pos = (self.read_pos + to_read) % self.buffer.len();

        to_read
    }

    /// Read all data from buffer (non-destructive)
    fn read_all(&self, buf: &mut [u8]) -> usize {
        let to_read = core::cmp::min(buf.len(), self.count);

        for (i, item) in buf.iter_mut().enumerate().take(to_read) {
            let pos = (self.start_pos + i) % self.buffer.len();
            *item = self.buffer[pos];
        }

        to_read
    }

    /// Clear the buffer completely
    fn clear(&mut self) {
        self.start_pos = 0;
        self.read_pos = 0;
        self.count = 0;
        // Don't reset total_written as it's cumulative statistics
    }

    /// Get size of unread data (data available for destructive read)
    fn unread_size(&self) -> usize {
        self.unread_count()
    }

    /// Helper: calculate unread count for destructive reads
    fn unread_count(&self) -> usize {
        if self.count == 0 {
            return 0;
        }

        // Calculate how much data is available from read_pos to end of valid data
        let end_pos = (self.start_pos + self.count - 1) % self.buffer.len();

        if end_pos >= self.read_pos {
            end_pos - self.read_pos + 1
        } else {
            // Wrapped around
            (self.buffer.len() - self.read_pos) + end_pos + 1
        }
    }

    /// Get buffer capacity
    fn buffer_size(&self) -> usize {
        self.buffer.len()
    }
}

/// Global kernel log buffer (lazily initialized)
static KERNEL_LOG_BUFFER: Once<SpinLock<KernelLogBuffer>> = Once::new();

/// dmesg_restrict setting - when true, only privileged users can read kernel messages
static DMESG_RESTRICT: SpinLock<bool> = SpinLock::new(false);

/// Get or initialize the kernel log buffer
fn get_kernel_log_buffer() -> &'static SpinLock<KernelLogBuffer> {
    KERNEL_LOG_BUFFER.call_once(|| SpinLock::new(KernelLogBuffer::new()))
}

/// Add a message to the kernel log buffer
pub fn add_to_kernel_log(level: log::Level, message: &str) {
    let mut buffer = get_kernel_log_buffer().lock();

    // Add Linux kernel-style log level prefix
    let level_prefix = match level {
        log::Level::Error => format!("<{}>", KERN_ERR),
        log::Level::Warn => format!("<{}>", KERN_WARNING),
        log::Level::Info => format!("<{}>", KERN_INFO),
        log::Level::Debug => format!("<{}>", KERN_DEBUG),
        log::Level::Trace => format!("<{}>", KERN_DEBUG),
    };

    // Use seconds.microseconds format
    let timestamp = ostd::timer::Jiffies::elapsed().as_duration();
    let secs = timestamp.as_secs();
    let micros = timestamp.subsec_micros();

    let formatted = format!("{level_prefix}[{:>5}.{:06}] {}\n", secs, micros, message);
    buffer.append(formatted.as_bytes());
}

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
                clear_kernel_log();
                return Ok(SyscallReturn::Return(0));
            }
            let bytes_read = read_kernel_log(buf_ptr, len as usize, ctx, false)?;
            clear_kernel_log();
            Ok(bytes_read)
        }

        SYSLOG_ACTION_CLEAR => {
            check_syslog_permission_detailed(ctx, action)?;
            clear_kernel_log();
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
            let size = get_unread_size();
            Ok(SyscallReturn::Return(size as isize))
        }

        SYSLOG_ACTION_SIZE_BUFFER => {
            check_syslog_permission_detailed(ctx, action)?;
            let size = get_buffer_size();
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
    let bytes_read = {
        let mut buffer = get_kernel_log_buffer().lock();
        if destructive {
            buffer.read_destructive(&mut temp_buf)
        } else {
            buffer.read_all(&mut temp_buf)
        }
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

/// Clear the kernel log buffer
fn clear_kernel_log() {
    let mut buffer = get_kernel_log_buffer().lock();
    buffer.clear();
}

/// Set console log level
fn set_console_log_level(level: i32) -> Result<SyscallReturn> {
    aster_logger::set_console_log_level(level);
    Ok(SyscallReturn::Return(0))
}

/// Get size of unread data in kernel log buffer
fn get_unread_size() -> usize {
    let buffer = get_kernel_log_buffer().lock();
    buffer.unread_size()
}

/// Get total size of kernel log buffer
fn get_buffer_size() -> usize {
    let buffer = get_kernel_log_buffer().lock();
    buffer.buffer_size()
}
