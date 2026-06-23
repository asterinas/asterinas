// SPDX-License-Identifier: MPL-2.0

//! Provides the kernel log buffer and Linux-compatible log controls.
//!
//! This module stores formatted kernel log records in a fixed-size ring buffer
//! and exposes methods used by Linux `syslog` and `dmesg` operations.

use core::{
    fmt::Write,
    num::Wrapping,
    sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    time::Duration,
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use int_to_c_enum::TryFromInt;
use ostd::{
    log::{Level, Record},
    mm::{Fallible, FallibleVmWrite, VmReader, VmWriter},
    sync::{LocalIrqDisabled, SpinLock, WaitQueue},
};
use ring_buffer::RingBuffer;
use spin::Once;

/// Size of the kernel log ring buffer (64 KB).
///
/// This matches the default `CONFIG_LOG_BUF_SHIFT=16` in Linux.
/// The buffer stores formatted log messages including timestamps.
const LOG_BUFFER_CAPACITY: usize = 64 * 1024;

/// Maximum size of a single formatted log record.
///
/// Linux bounds formatted printk records with `PRINTKRB_RECORD_MAX`, currently
/// 1024 bytes. This limit applies before the record is appended to the ring
/// buffer, so an oversized record cannot evict the whole log buffer by itself.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/kernel/printk/internal.h#L52>
const MAX_RECORD_SIZE: usize = 1024;

/// Minimum Linux console log level accepted by syslog console controls.
///
/// Linux clamps `SYSLOG_ACTION_CONSOLE_LEVEL` to `minimum_console_loglevel`
/// and uses the same level for `SYSLOG_ACTION_CONSOLE_OFF`.
pub const MINIMUM_CONSOLE_LOG_LEVEL: LinuxConsoleLogLevel = LinuxConsoleLogLevel::Emerg;

/// Linux console log level.
///
/// Controls which log messages are broadcast to the console. Only messages
/// with a priority level less than the console log level will be displayed.
/// For example, setting the console level to `Error` (4) will display messages
/// of levels `Emerg`, `Alert`, `Crit`, and `Error`.
///
/// These values correspond to Linux's console log level as documented in
/// `man 2 syslog` and used by the `SYSLOG_ACTION_CONSOLE_LEVEL` action.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, TryFromInt)]
pub enum LinuxConsoleLogLevel {
    /// Allow messages with priority `KERN_EMERG` (0) or higher.
    Emerg = 1,
    /// Allow messages with priority `KERN_ALERT` (1) or higher.
    Alert = 2,
    /// Allow messages with priority `KERN_CRIT` (2) or higher.
    Crit = 3,
    /// Allow messages with priority `KERN_ERR` (3) or higher.
    Err = 4,
    /// Allow messages with priority `KERN_WARNING` (4) or higher.
    Warn = 5,
    /// Allow messages with priority `KERN_NOTICE` (5) or higher.
    Notice = 6,
    /// Allow messages with priority `KERN_INFO` (6) or higher.
    Info = 7,
    /// Allow messages with priority `KERN_DEBUG` (7) or higher (all messages).
    Debug = 8,
}

impl LinuxConsoleLogLevel {
    /// Checks if a log level should be printed at this console log level.
    fn should_print(self, level: Level) -> bool {
        (level as u8) < (self as u8)
    }
}

impl From<LinuxConsoleLogLevel> for u8 {
    fn from(level: LinuxConsoleLogLevel) -> Self {
        level as u8
    }
}

define_atomic_version_of_integer_like_type!(LinuxConsoleLogLevel, try_from = true, {
    /// An atomic version of `LinuxConsoleLogLevel`.
    #[derive(Debug)]
    struct AtomicLinuxConsoleLogLevel(AtomicU8);
});

static KLOG: Once<KernelLog> = Once::new();

/// Returns the global kernel log.
pub fn klog() -> &'static KernelLog {
    KLOG.call_once(KernelLog::new)
}

/// Stores kernel log records and Linux-compatible log controls.
///
/// `KernelLog` owns the fixed-size ring buffer used by `dmesg`/`syslog`,
/// console log-level state, and the `dmesg_restrict` access-control flag.
/// Callers should obtain the global instance with [`klog`].
pub struct KernelLog {
    buffer: SpinLock<RingBuffer<u8>, LocalIrqDisabled>,
    dmesg_restrict: AtomicBool,
    /// First byte visible to non-destructive latest-pick operations.
    pick_head: AtomicUsize,
    /// Current console log level.
    console_level: AtomicLinuxConsoleLogLevel,
    /// Saved console log level for restore after disable/enable.
    saved_console_level: SpinLock<Option<LinuxConsoleLogLevel>, LocalIrqDisabled>,
    waitq: WaitQueue,
}

impl KernelLog {
    fn new() -> Self {
        Self {
            buffer: SpinLock::new(RingBuffer::new(LOG_BUFFER_CAPACITY)),
            dmesg_restrict: AtomicBool::new(false),
            pick_head: AtomicUsize::new(0),
            // TODO: Make the initial console level be configurable.
            console_level: AtomicLinuxConsoleLogLevel::new(LinuxConsoleLogLevel::Err),
            saved_console_level: SpinLock::new(None),
            waitq: WaitQueue::new(),
        }
    }

    /// Appends a log record to the kernel log buffer.
    ///
    /// The record is formatted with a timestamp prefix before being stored.
    /// This method does not allocate memory and can be called from interrupt
    /// context.
    pub fn append(&self, record: &Record, timestamp: &Duration) {
        let bytes_written = {
            let mut buffer = self.buffer.lock();
            let mut formatter = buffer.formatter().limit(MAX_RECORD_SIZE);
            let _ = writeln!(
                formatter,
                "[{:>6}.{:03}] {:<6}: {}{}",
                timestamp.as_secs(),
                timestamp.subsec_millis(),
                record.level(),
                record.prefix(),
                record.args()
            );
            formatter.bytes_written()
        };

        if bytes_written > 0 {
            self.waitq.wake_all();
        }
    }

    /// Reads log messages from the kernel log buffer destructively.
    ///
    /// Returns the number of bytes written into `writer`.
    pub fn read(&self, writer: &mut VmWriter<'_, Fallible>) -> ostd::Result<usize> {
        let mut total_written = 0;
        let mut temp_buffer = [0u8; MAX_RECORD_SIZE];

        while writer.has_avail() {
            let bytes_read = {
                let mut buffer = self.buffer.lock();
                let read_len = buffer.len().min(temp_buffer.len()).min(writer.avail());
                if read_len == 0 {
                    return Ok(total_written);
                }

                let read_start = buffer.head();
                let read_end = read_start + Wrapping(read_len);
                let mut temp_writer = VmWriter::from(&mut temp_buffer[..read_len]).to_fallible();
                // The writer points to a valid kernel stack buffer, so the
                // fallible copy cannot fault.
                let written = buffer
                    .pick_range(read_start..read_end, &mut temp_writer)
                    .unwrap();
                buffer.commit_read(written);
                written
            };

            let mut temp_reader = VmReader::from(&temp_buffer[..bytes_read]);
            let bytes_written = match writer.write_fallible(&mut temp_reader) {
                Ok(written) => written,
                Err((err, 0)) if total_written == 0 => return Err(err),
                Err((_, written)) => written,
            };
            total_written += bytes_written;

            if bytes_written < bytes_read {
                return Ok(total_written);
            }
        }

        Ok(total_written)
    }

    /// Picks the last log messages from the kernel log buffer.
    ///
    /// Writes the last available log bytes that fit in `writer`, ending at the
    /// current buffer tail. The data remains in the buffer after picking.
    pub fn pick_latest(&self, writer: &mut VmWriter<'_, Fallible>) -> ostd::Result<usize> {
        self.pick_latest_impl(writer, false)
    }

    /// Picks the last log messages and clears the visible range.
    ///
    /// Writes the last visible log bytes that fit in `writer`, ending at the
    /// current buffer tail. The pick head is advanced under the same buffer
    /// lock used for picking, so messages appended after the pick are not
    /// accidentally hidden.
    pub fn pick_latest_and_clear(
        &self,
        writer: &mut VmWriter<'_, Fallible>,
    ) -> ostd::Result<usize> {
        self.pick_latest_impl(writer, true)
    }

    fn pick_latest_impl(
        &self,
        writer: &mut VmWriter<'_, Fallible>,
        clear_after_pick: bool,
    ) -> ostd::Result<usize> {
        let mut total_written = 0;
        let mut temp_buffer = [0u8; MAX_RECORD_SIZE];
        let (mut next_read_start, read_end, mut remaining) = {
            let buffer = self.buffer.lock();

            let tail = buffer.tail();
            let len = (tail - buffer.head()).0;
            let len_from_pick_head = {
                let pick_head = Wrapping(self.pick_head.load(Ordering::Relaxed));
                (tail - pick_head).0
            };

            let pick_len = len.min(len_from_pick_head).min(writer.avail());
            if pick_len == 0 {
                return Ok(total_written);
            }

            if clear_after_pick {
                self.pick_head.store(tail.0, Ordering::Relaxed);
            }

            (tail - Wrapping(pick_len), tail, pick_len)
        };

        while remaining > 0 && writer.has_avail() {
            let bytes_read = {
                let buffer = self.buffer.lock();
                let head = buffer.head();
                let len = (buffer.tail() - head).0;

                let read_end_offset = (read_end - head).0;
                // The chosen end has fallen behind the current readable
                // range, so none of the remaining snapshot bytes survived.
                if read_end_offset > len {
                    return Ok(total_written);
                }

                let read_start_offset = (next_read_start - head).0;
                // The next byte to read was overwritten. Best-effort reads
                // skip the lost prefix and resume from the oldest live byte.
                if read_start_offset > len {
                    next_read_start = head;
                    remaining = read_end_offset;
                    // The lost prefix reached the original end of this read.
                    if remaining == 0 {
                        return Ok(total_written);
                    }
                }

                let chunk_len = remaining.min(temp_buffer.len()).min(writer.avail());
                let chunk_end = next_read_start + Wrapping(chunk_len);
                let mut temp_writer = VmWriter::from(&mut temp_buffer[..chunk_len]).to_fallible();
                // The writer points to a valid kernel stack buffer, so the
                // fallible copy cannot fault.
                buffer
                    .pick_range(next_read_start..chunk_end, &mut temp_writer)
                    .unwrap()
            };

            let mut temp_reader = VmReader::from(&temp_buffer[..bytes_read]);
            let bytes_written = match writer.write_fallible(&mut temp_reader) {
                Ok(written) => written,
                Err((err, 0)) if total_written == 0 => return Err(err),
                Err((_, written)) => written,
            };
            total_written += bytes_written;

            if bytes_written < bytes_read {
                return Ok(total_written);
            }

            remaining -= bytes_written;
            next_read_start += Wrapping(bytes_written);
        }

        Ok(total_written)
    }

    /// Marks the current buffer position as the next pick head.
    ///
    /// After this call, [`Self::pick_latest`] only returns messages logged after
    /// this mark, but the data is not removed from the buffer.
    pub fn mark_clear(&self) {
        let buffer = self.buffer.lock();
        self.pick_head.store(buffer.tail().0, Ordering::Relaxed);
    }

    /// Returns the number of unread bytes in the kernel log buffer.
    pub fn size_unread(&self) -> usize {
        self.buffer.lock().len()
    }

    /// Returns the total capacity of the kernel log buffer in bytes.
    pub fn capacity(&self) -> usize {
        LOG_BUFFER_CAPACITY
    }

    /// Returns the current value of the `dmesg_restrict` flag.
    pub fn dmesg_restrict(&self) -> bool {
        self.dmesg_restrict.load(Ordering::Relaxed)
    }

    /// Sets the `dmesg_restrict` flag.
    ///
    /// When enabled, reading all log messages requires `CAP_SYSLOG` or
    /// `CAP_SYS_ADMIN`.
    pub fn set_dmesg_restrict(&self, val: bool) {
        self.dmesg_restrict.store(val, Ordering::Relaxed);
    }

    /// Sets the console log level.
    ///
    /// Returns the previous console log level.
    pub fn set_console_level(&self, level: LinuxConsoleLogLevel) -> LinuxConsoleLogLevel {
        let mut saved = self.saved_console_level.lock();
        let old = self.console_level.load(Ordering::Relaxed);
        self.console_level.store(level, Ordering::Relaxed);
        *saved = None;
        old
    }

    /// Disables console output by setting the log level to the Linux minimum.
    ///
    /// The previous log level is saved and can be restored with
    /// [`Self::enable_console`]. Returns the previous console log level.
    pub fn disable_console(&self) -> LinuxConsoleLogLevel {
        let mut saved = self.saved_console_level.lock();
        let old = self.console_level.load(Ordering::Relaxed);
        self.console_level
            .store(MINIMUM_CONSOLE_LOG_LEVEL, Ordering::Relaxed);
        if saved.is_none() {
            *saved = Some(old);
        }
        old
    }

    /// Restores the console log level that was saved by [`Self::disable_console`].
    ///
    /// Returns the restored console log level.
    pub fn enable_console(&self) -> LinuxConsoleLogLevel {
        let mut saved = self.saved_console_level.lock();
        if let Some(prev) = saved.take() {
            self.console_level.store(prev, Ordering::Relaxed);
            prev
        } else {
            self.console_level()
        }
    }

    /// Returns the current console log level.
    pub fn console_level(&self) -> LinuxConsoleLogLevel {
        self.console_level.load(Ordering::Relaxed)
    }

    /// Checks if a log message at the given level should be printed to console.
    pub fn should_print(&self, level: Level) -> bool {
        self.console_level().should_print(level)
    }

    /// Returns whether the destructive-read side of the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.lock().is_empty()
    }

    /// Returns the wait queue notified when new log bytes are appended.
    pub fn wait_queue(&self) -> &WaitQueue {
        &self.waitq
    }
}
