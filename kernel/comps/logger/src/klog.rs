use alloc::sync::Arc;
use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
    time::Duration,
};

use log::{Level, LevelFilter, Record};
use ostd::{
    mm::VmIo,
    sync::{Once, SpinLock, WaitQueue},
};
use ring_buffer::RingBuffer;

/// Size of the kernel log ring buffer (64 KB).
///
/// This matches the default `CONFIG_LOG_BUF_SHIFT=16` in Linux.
/// The buffer stores formatted log messages including timestamps.
const LOG_BUFFER_CAPACITY: usize = 64 * 1024;

/// Maximum size of a single formatted log message.
///
/// Log messages longer than this will be truncated. This is a stack-allocated
/// scratch buffer used to format messages without heap allocation, which is
/// important for logging in low-memory or allocator-internal contexts.
const FORMAT_BUF_CAPACITY: usize = 512;

/// Chunk size for copying data to/from user space.
///
/// Reading and writing are done in chunks to avoid holding locks for too long
/// and to limit stack usage.
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

    /// Converts the console log level to a `log::LevelFilter`.
    fn to_level_filter(self) -> LevelFilter {
        match self {
            Self::Off => LevelFilter::Off,
            Self::Emerg | Self::Alert | Self::Crit | Self::Error => LevelFilter::Error,
            Self::Warn => LevelFilter::Warn,
            Self::Notice | Self::Info => LevelFilter::Info,
            Self::Debug => LevelFilter::Debug,
        }
    }

    /// Checks if a log level should be printed at this console log level.
    fn should_print(self, level: Level) -> bool {
        match self {
            Self::Off => false,
            Self::Emerg | Self::Alert | Self::Crit | Self::Error => matches!(level, Level::Error),
            Self::Warn => matches!(level, Level::Error | Level::Warn),
            Self::Notice | Self::Info => matches!(level, Level::Error | Level::Warn | Level::Info),
            Self::Debug => !matches!(level, Level::Trace),
        }
    }
}

static KLOG: Once<Arc<KernelLog>> = Once::new();

/// Initializes the kernel log buffer.
///
/// This function must be called before any logging operations.
pub fn init_klog() {
    KLOG.call_once(|| Arc::new(KernelLog::new()));
}

fn klog() -> &'static Arc<KernelLog> {
    KLOG.get()
        .expect("klog not initialized; call init_klog() first")
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
        "[{:>6}.{:03}] {:<5}: {}\n",
        secs,
        millis,
        record.level(),
        record.args()
    );

    if !writer.is_empty() {
        klog().append(writer.as_bytes());
    }
}

/// Checks if a log message at the given level should be printed to console.
pub fn should_print(level: Level) -> bool {
    klog().should_print(level)
}

/// Returns the current console log level.
pub fn console_level() -> LinuxConsoleLogLevel {
    klog().get_console_level()
}

/// Sets the console log level.
///
/// Returns the previous console log level.
pub fn console_set_level(level: LinuxConsoleLogLevel) -> LinuxConsoleLogLevel {
    klog().set_console_level(level, false)
}

/// Disables console output by setting the log level to `Off`.
///
/// The previous log level is saved and can be restored with `console_on()`.
/// Returns the previous console log level.
pub fn console_off() -> LinuxConsoleLogLevel {
    klog().set_console_level(LinuxConsoleLogLevel::Off, true)
}

/// Restores the console log level that was saved by `console_off()`.
///
/// Returns the restored console log level.
pub fn console_on() -> LinuxConsoleLogLevel {
    klog().restore_console_level()
}

/// Reads log messages from the kernel log buffer (destructive).
///
/// This is a blocking read that removes data from the buffer as it is read.
/// Returns the number of bytes read into `dst`.
pub fn klog_read(dst: &mut [u8]) -> usize {
    klog().read(dst)
}

/// Reads log messages from the kernel log buffer (non-destructive).
///
/// Reads up to `window_len` bytes starting at `offset` within the available
/// log data. The data remains in the buffer after reading.
/// Returns the number of bytes read into `dst`.
pub fn klog_read_all(dst: &mut [u8], offset: usize, window_len: usize) -> usize {
    klog().read_all(dst, offset, window_len)
}

/// Marks the current buffer position as cleared.
///
/// After this call, `klog_read_all` will only return messages logged after
/// the clear operation, but the data is not actually removed from the buffer.
pub fn mark_clear() {
    klog().mark_clear();
}

/// Returns the number of unread bytes in the kernel log buffer.
pub fn klog_size_unread() -> usize {
    klog().size_unread()
}

/// Returns the total capacity of the kernel log buffer in bytes.
pub fn klog_capacity() -> usize {
    LOG_BUFFER_CAPACITY
}

/// Checks if reading all log messages requires `CAP_SYSLOG` or `CAP_SYS_ADMIN`.
///
/// When `dmesg_restrict` is enabled, unprivileged users cannot read the
/// kernel log buffer via `SYSLOG_ACTION_READ_ALL`.
pub fn read_all_requires_cap() -> bool {
    klog().dmesg_restrict()
}

/// Gets the current value of the `dmesg_restrict` flag.
pub fn dmesg_restrict_get() -> bool {
    klog().dmesg_restrict.load(Ordering::Relaxed)
}

/// Sets the `dmesg_restrict` flag.
///
/// When enabled, reading all log messages requires `CAP_SYSLOG` or `CAP_SYS_ADMIN`.
pub fn dmesg_restrict_set(val: bool) {
    klog().dmesg_restrict.store(val, Ordering::Relaxed);
}

struct KernelLog {
    buffer: SpinLock<RingBuffer<u8>>,
    clear_tail: SpinLock<usize>,
    dmesg_restrict: AtomicBool,
    /// Current console log level (stored as raw u8 for atomic access).
    console_level: AtomicU8,
    /// Saved console log level for restore after console_off/console_on.
    saved_console_level: SpinLock<Option<LinuxConsoleLogLevel>>,
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
            }

            // Drop oldest data if needed.
            let free = buf.free_len();
            let need_drop = bytes.len().saturating_sub(free);
            if need_drop > 0 {
                buf.commit_read(need_drop);
                self.bump_clear_tail(&mut buf);
            }

            buf.push_slice(bytes)
                .expect("push_slice must succeed after drop");
        }

        // Wake up blocked readers once new data arrives.
        self.waitq.wake_all();
    }

    fn bump_clear_tail(&self, buf: &mut RingBuffer<u8>) {
        let mut clear_tail = self.clear_tail.lock();
        let head = buf.head().0;
        if *clear_tail < head {
            *clear_tail = head;
        }
    }

    fn read(&self, dst: &mut [u8]) -> usize {
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
                self.bump_clear_tail(&mut buf);
                take
            };
            copied += chunk;
        }
        copied
    }

    fn read_all(&self, dst: &mut [u8], offset: usize, window_len: usize) -> usize {
        let mut copied = 0;
        while copied < dst.len() {
            let (take, start) = {
                let buf = self.buffer.lock();
                let head = buf.head().0;
                let tail = buf.tail().0;
                let base = core::cmp::max(head, *self.clear_tail.lock());
                let available = tail.saturating_sub(base);
                if available == 0 {
                    return copied;
                }
                let window = core::cmp::min(available, window_len);
                if offset + copied >= window {
                    return copied;
                }
                let remain = window - (offset + copied);
                let take = core::cmp::min(core::cmp::min(dst.len() - copied, COPY_CHUNK), remain);
                let start = (tail - window) + offset + copied;
                (take, start)
            };

            copy_from(&self.buffer.lock(), start, &mut dst[copied..copied + take]);
            copied += take;
        }
        copied
    }

    fn mark_clear(&self) {
        let buf = self.buffer.lock();
        let mut clear_tail = self.clear_tail.lock();
        *clear_tail = buf.tail().0;
    }

    fn size_unread(&self) -> usize {
        self.buffer.lock().len()
    }

    fn dmesg_restrict(&self) -> bool {
        self.dmesg_restrict.load(Ordering::Relaxed)
    }

    fn set_console_level(
        &self,
        level: LinuxConsoleLogLevel,
        save_old: bool,
    ) -> LinuxConsoleLogLevel {
        let mut saved = self.saved_console_level.lock();
        let old_raw = self.console_level.swap(level as u8, Ordering::SeqCst);
        // SAFETY: We only store valid LinuxConsoleLogLevel values.
        let old =
            LinuxConsoleLogLevel::from_raw(old_raw as i32).unwrap_or(LinuxConsoleLogLevel::Info);
        if save_old && saved.is_none() {
            *saved = Some(old);
        }
        old
    }

    fn restore_console_level(&self) -> LinuxConsoleLogLevel {
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

    fn should_print(&self, level: Level) -> bool {
        self.get_console_level().should_print(level)
    }

    fn wait_nonempty(&self) {
        self.waitq
            .wait_until(|| (!self.buffer.lock().is_empty()).then_some(()));
    }
}

pub fn klog_wait_nonempty() {
    klog().wait_nonempty();
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
