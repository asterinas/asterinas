// SPDX-License-Identifier: MPL-2.0

//! Kernel log buffer (`klog`) backing `dmesg(1)` and `syslog(2)`.
//!
//! `KernelLog` stores formatted records in a fixed-capacity ring buffer; the
//! oldest records are evicted on overflow. Console output is gated by a
//! configurable level independent of buffer retention.
//!
//! # Locking
//!
//! Lock order on `KernelLog`: `buffer` -> `clear_tail`. `saved_console_level`
//! is independent. All locks are IRQ-safe because the logger fires from
//! interrupt context.
//!
//! # Invariants
//!
//! - `clear_tail` is monotonic and always satisfies `head <= clear_tail <= tail`
//!   (modulo wraparound of the cumulative `usize` counters).
//! - `console_level` is always a valid `LinuxConsoleLogLevel` discriminant.

use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
    time::Duration,
};

use ostd::{
    log::{Level, Record},
    mm::VmIo,
    sync::{LocalIrqDisabled, SpinLock, WaitQueue},
};
use ring_buffer::RingBuffer;
use spin::Once;

/// Size of the kernel log ring buffer (64 KB).
///
/// This matches the default `CONFIG_LOG_BUF_SHIFT=16` in Linux.
/// The buffer stores formatted log messages including timestamps.
pub const LOG_BUFFER_CAPACITY: usize = 64 * 1024;

/// Maximum size of a single formatted log message.
///
/// Log messages longer than this will be truncated. This is a stack-allocated
/// scratch buffer used to format messages without heap allocation, which is
/// important for logging in low-memory or allocator-internal contexts.
const FORMAT_BUF_CAPACITY: usize = 512;

/// Chunk size for copying data between kernel and user space.
///
/// Reading is done in chunks to limit stack usage and avoid holding
/// locks for too long.
const COPY_CHUNK: usize = 512;

/// Linux console log level.
///
/// Controls which log messages are broadcast to the console. Only messages
/// with a priority level less than the console log level will be displayed.
/// For example, setting the console level to `Error` (4) will display messages
/// of levels `Emerg`, `Alert`, `Crit`, and `Error`.
///
/// These values correspond to Linux's console log level as documented in
/// `man 2 syslog` and used by the `SYSLOG_ACTION_CONSOLE_LEVEL` action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LinuxConsoleLogLevel {
    /// Suppress all console output.
    Off = 0,
    /// Allow messages with priority `KERN_EMERG` (0) or higher.
    Emerg = 1,
    /// Allow messages with priority `KERN_ALERT` (1) or higher.
    Alert = 2,
    /// Allow messages with priority `KERN_CRIT` (2) or higher.
    Crit = 3,
    /// Allow messages with priority `KERN_ERR` (3) or higher.
    Error = 4,
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
    /// Creates a console log level from a raw integer value.
    ///
    /// Returns `None` if the value is out of the valid range (0-8).
    pub fn from_raw(raw: i32) -> Option<Self> {
        match raw {
            0 => Some(Self::Off),
            1 => Some(Self::Emerg),
            2 => Some(Self::Alert),
            3 => Some(Self::Crit),
            4 => Some(Self::Error),
            5 => Some(Self::Warn),
            6 => Some(Self::Notice),
            7 => Some(Self::Info),
            8 => Some(Self::Debug),
            _ => None,
        }
    }

    /// Checks if a log level should be printed at this console log level.
    fn should_print(self, level: Level) -> bool {
        (level as u8) < (self as u8)
    }
}

static KLOG: Once<KernelLog> = Once::new();

/// Returns a reference to the global [`KernelLog`].
///
/// Initializes the kernel log buffer on first call.
pub fn klog() -> &'static KernelLog {
    KLOG.call_once(KernelLog::new)
}

/// Initializes the kernel log buffer.
///
/// This function must be called before any logging operations.
pub fn init_klog() {
    KLOG.call_once(KernelLog::new);
}

struct FixedBuf<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> FixedBuf<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Write for FixedBuf<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let space = self.buf.len().saturating_sub(self.len);
        if space == 0 {
            return Ok(());
        }

        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(space);
        self.buf[self.len..self.len + copy_len].copy_from_slice(&bytes[..copy_len]);
        self.len += copy_len;
        Ok(())
    }
}

/// Appends a log record to the kernel log buffer.
///
/// The record is formatted with a timestamp prefix before being stored.
/// This function does not allocate memory and uses a fixed-size scratch buffer.
pub fn append_log(record: &Record, timestamp: &Duration) {
    let mut scratch = [0u8; FORMAT_BUF_CAPACITY];
    let mut writer = FixedBuf::new(&mut scratch);

    let secs = timestamp.as_secs();
    let millis = timestamp.subsec_millis();
    let _ = writeln!(
        writer,
        "[{:>6}.{:03}] {:<6}: {}{}",
        secs,
        millis,
        record.level(),
        record.prefix(),
        record.args()
    );

    if !writer.is_empty() {
        klog().append(writer.as_bytes());
    }
}

/// The kernel log buffer.
///
/// Stores formatted log records in a fixed-capacity ring buffer, manages
/// console log levels, and provides read access for `syslog(2)` and `/proc/sys/kernel/dmesg_restrict`.
pub struct KernelLog {
    buffer: SpinLock<RingBuffer<u8>, LocalIrqDisabled>,
    clear_tail: SpinLock<usize, LocalIrqDisabled>,
    dmesg_restrict: AtomicBool,
    /// Current console log level (stored as raw u8 for atomic access).
    console_level: AtomicU8,
    /// Saved console log level for restore after console_off/console_on.
    saved_console_level: SpinLock<Option<LinuxConsoleLogLevel>, LocalIrqDisabled>,
    waitq: WaitQueue,
}

impl KernelLog {
    fn new() -> Self {
        Self {
            buffer: SpinLock::new(RingBuffer::new(LOG_BUFFER_CAPACITY)),
            clear_tail: SpinLock::new(0),
            dmesg_restrict: AtomicBool::new(false),
            console_level: AtomicU8::new(LinuxConsoleLogLevel::Info as u8),
            saved_console_level: SpinLock::new(None),
            waitq: WaitQueue::new(),
        }
    }

    fn append(&self, mut bytes: &[u8]) {
        {
            let mut buf = self.buffer.lock();
            let cap = buf.capacity();

            if bytes.len() > cap {
                bytes = &bytes[bytes.len() - cap..];
                buf.clear();
                *self.clear_tail.lock() = 0;
            }

            // Drop oldest data if needed.
            let free = buf.free_len();
            let need_drop = bytes.len().saturating_sub(free);
            if need_drop > 0 {
                buf.commit_read(need_drop);
                self.bump_clear_tail(&buf);
            }

            buf.push_slice(bytes)
                .expect("push_slice must succeed after drop");
        }

        // The `buffer` lock must be dropped before waking — the waiters' re-check
        // in `wait_nonempty` re-acquires it.
        self.waitq.wake_all();
    }

    fn bump_clear_tail(&self, buf: &RingBuffer<u8>) {
        let mut clear_tail = self.clear_tail.lock();
        let head = buf.head().0;
        if *clear_tail < head {
            *clear_tail = head;
        }
    }

    /// Reads and removes log messages from the buffer (destructive).
    ///
    /// Returns the number of bytes read into `dst`.
    pub fn read(&self, dst: &mut [u8]) -> usize {
        let mut copied = 0;
        while copied < dst.len() {
            let chunk = {
                let mut buf = self.buffer.lock();
                let available = buf.len();
                if available == 0 {
                    break;
                }
                let take =
                    core::cmp::min(core::cmp::min(dst.len() - copied, COPY_CHUNK), available);
                copy_from(&buf, buf.head().0, &mut dst[copied..copied + take]);
                buf.commit_read(take);
                self.bump_clear_tail(&buf);
                take
            };
            copied += chunk;
        }
        copied
    }

    /// Reads log messages without removing them (non-destructive).
    ///
    /// Reads up to `window_len` bytes starting at `offset` within the available
    /// log data, pinned to the snapshot `at_tail` for cross-chunk consistency.
    /// Returns the number of bytes read into `dst`.
    pub fn read_all(&self, dst: &mut [u8], offset: usize, window_len: usize, at_tail: usize) -> usize {
        let buf = self.buffer.lock();
        let base = core::cmp::max(buf.head().0, *self.clear_tail.lock());
        let available = at_tail.saturating_sub(base);
        let window = core::cmp::min(available, window_len);
        if offset >= window {
            return 0;
        }
        let take = core::cmp::min(dst.len(), window - offset);
        let start = (at_tail - window) + offset;
        copy_from(&buf, start, &mut dst[..take]);
        take
    }

    /// Returns the current tail position for snapshot-based reads.
    ///
    /// The returned value can be passed to [`read_all`](Self::read_all) to
    /// ensure all chunks within a single syscall observe a consistent view.
    pub fn snapshot_tail(&self) -> usize {
        self.buffer.lock().tail().0
    }

    /// Marks the current buffer position as cleared.
    ///
    /// After this call, [`read_all`](Self::read_all) will only return messages
    /// logged after the clear operation.
    pub fn mark_clear(&self) {
        let buf = self.buffer.lock();
        let mut clear_tail = self.clear_tail.lock();
        *clear_tail = buf.tail().0;
    }

    /// Returns the number of unread bytes in the kernel log buffer.
    pub fn size_unread(&self) -> usize {
        let buf = self.buffer.lock();
        let base = core::cmp::max(buf.head().0, *self.clear_tail.lock());
        buf.tail().0.saturating_sub(base)
    }

    /// Returns whether `dmesg_restrict` is enabled.
    pub fn dmesg_restrict(&self) -> bool {
        self.dmesg_restrict.load(Ordering::Relaxed)
    }

    /// Sets the `dmesg_restrict` flag.
    ///
    /// When enabled, reading all log messages requires `CAP_SYSLOG` or `CAP_SYS_ADMIN`.
    pub fn set_dmesg_restrict(&self, val: bool) {
        self.dmesg_restrict.store(val, Ordering::Relaxed);
    }

    /// Sets the console log level.
    ///
    /// Returns the previous console log level.
    pub fn set_console_level(&self, level: LinuxConsoleLogLevel) -> LinuxConsoleLogLevel {
        let old_raw = self.console_level.swap(level as u8, Ordering::SeqCst);
        // `console_level` is only ever written from `LinuxConsoleLogLevel as u8`,
        // so `from_raw` never fails; `unwrap_or(Info)` is defensive.
        LinuxConsoleLogLevel::from_raw(old_raw as i32).unwrap_or(LinuxConsoleLogLevel::Info)
    }

    /// Disables console output by setting the log level to `Off`.
    ///
    /// The previous log level is saved and can be restored with
    /// [`restore_console_level`](Self::restore_console_level).
    /// Returns the previous console log level.
    pub fn disable_console(&self) -> LinuxConsoleLogLevel {
        let mut saved = self.saved_console_level.lock();
        let old_raw = self.console_level.swap(LinuxConsoleLogLevel::Off as u8, Ordering::SeqCst);
        let old =
            LinuxConsoleLogLevel::from_raw(old_raw as i32).unwrap_or(LinuxConsoleLogLevel::Info);
        if saved.is_none() {
            *saved = Some(old);
        }
        old
    }

    /// Restores the console log level saved by
    /// [`disable_console`](Self::disable_console).
    ///
    /// Returns the restored console log level.
    pub fn restore_console_level(&self) -> LinuxConsoleLogLevel {
        let mut saved = self.saved_console_level.lock();
        if let Some(prev) = saved.take() {
            self.console_level.store(prev as u8, Ordering::SeqCst);
            prev
        } else {
            self.get_console_level()
        }
    }

    fn get_console_level(&self) -> LinuxConsoleLogLevel {
        let raw = self.console_level.load(Ordering::SeqCst);
        LinuxConsoleLogLevel::from_raw(raw as i32).unwrap_or(LinuxConsoleLogLevel::Info)
    }

    /// Checks if a log message at the given level should be printed to console.
    pub fn should_print(&self, level: Level) -> bool {
        self.get_console_level().should_print(level)
    }

    /// Blocks until the kernel log buffer is non-empty.
    pub fn wait_nonempty(&self) {
        self.waitq
            .wait_until(|| (!self.buffer.lock().is_empty()).then_some(()));
    }
}

fn copy_from(rb: &RingBuffer<u8>, start: usize, dst: &mut [u8]) {
    let cap = rb.capacity();
    let offset = start & (cap - 1);
    if offset + dst.len() > cap {
        let first = cap - offset;
        rb.segment().read_slice(offset, &mut dst[..first]).unwrap();
        rb.segment().read_slice(0, &mut dst[first..]).unwrap();
    } else {
        rb.segment().read_slice(offset, dst).unwrap();
    }
}
