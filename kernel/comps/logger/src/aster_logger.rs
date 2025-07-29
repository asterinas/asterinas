// SPDX-License-Identifier: MPL-2.0

use alloc::{format, string::ToString, vec::Vec};

use log::{Metadata, Record};
use ostd::{sync::SpinLock, timer::Jiffies};

/// Console log level (default is 7 - show all messages)
static CONSOLE_LOG_LEVEL: SpinLock<i32> = SpinLock::new(7);

/// Kernel log buffer configuration
const KERNEL_LOG_BUFFER_SIZE: usize = 65536; // 64KB buffer

/// Kernel log priority levels (compatible with Linux)
const KERN_ERR: i32 = 3; // Error messages
const KERN_WARNING: i32 = 4; // Warning messages
const KERN_INFO: i32 = 6; // Informational messages
const KERN_DEBUG: i32 = 7; // Debug messages

/// Efficient circular kernel log buffer
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
        let buffer = Vec::with_capacity(KERNEL_LOG_BUFFER_SIZE);
        let mut buffer = buffer;
        buffer.resize(KERNEL_LOG_BUFFER_SIZE, 0);
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
static KERNEL_LOG_BUFFER: SpinLock<Option<KernelLogBuffer>> = SpinLock::new(None);

/// Get or initialize the kernel log buffer
fn get_kernel_log_buffer() -> &'static SpinLock<Option<KernelLogBuffer>> {
    &KERNEL_LOG_BUFFER
}

/// Add a message to the kernel log buffer
fn add_to_kernel_log(level: log::Level, message: &str) {
    let mut buffer_opt = get_kernel_log_buffer().lock();
    
    // Initialize buffer if not yet initialized
    if buffer_opt.is_none() {
        *buffer_opt = Some(KernelLogBuffer::new());
    }
    
    let buffer = buffer_opt.as_mut().unwrap();

    // Add Linux kernel-style log level prefix
    let level_prefix = match level {
        log::Level::Error => format!("<{}>", KERN_ERR),
        log::Level::Warn => format!("<{}>", KERN_WARNING),
        log::Level::Info => format!("<{}>", KERN_INFO),
        log::Level::Debug => format!("<{}>", KERN_DEBUG),
        log::Level::Trace => format!("<{}>", KERN_DEBUG),
    };

    // Use seconds.microseconds format
    let timestamp = Jiffies::elapsed().as_duration();
    let secs = timestamp.as_secs();
    let micros = timestamp.subsec_micros();

    let formatted = format!("{level_prefix}[{:>5}.{:06}] {}\n", secs, micros, message);
    buffer.append(formatted.as_bytes());
}

/// Update the console log level from external components
pub fn set_console_log_level(level: i32) {
    *CONSOLE_LOG_LEVEL.lock() = level;
    
    // Update the log crate's max level to ensure messages reach our logger
    // We need to set the log crate's level to the most permissive level
    // that corresponds to our console level to ensure all messages we want
    // can potentially reach our logger
    let log_level = match level {
        0 => log::LevelFilter::Off,    // No messages
        1..=3 => log::LevelFilter::Error,  // Only errors
        4 => log::LevelFilter::Warn,   // Warnings and errors
        5..=6 => log::LevelFilter::Info,   // Info, warnings, and errors
        7 => log::LevelFilter::Debug,  // Debug and above
        _ => log::LevelFilter::Trace,  // All messages
    };
    
    log::set_max_level(log_level);
}

/// Get current console log level
pub fn get_console_log_level() -> i32 {
    *CONSOLE_LOG_LEVEL.lock()
}

/// Convert log level to console priority level (matching Linux kernel levels)
fn log_level_to_console_level(level: log::Level) -> i32 {
    match level {
        log::Level::Error => 3, // KERN_ERR
        log::Level::Warn => 4,  // KERN_WARNING
        log::Level::Info => 6,  // KERN_INFO
        log::Level::Debug => 7, // KERN_DEBUG
        log::Level::Trace => 8, // KERN_DEBUG (highest verbosity)
    }
}

/// Functions for syslog syscall integration
pub fn read_kernel_log_destructive(buf: &mut [u8]) -> usize {
    let mut buffer_opt = get_kernel_log_buffer().lock();
    if let Some(buffer) = buffer_opt.as_mut() {
        buffer.read_destructive(buf)
    } else {
        0
    }
}

pub fn read_kernel_log_all(buf: &mut [u8]) -> usize {
    let buffer_opt = get_kernel_log_buffer().lock();
    if let Some(buffer) = buffer_opt.as_ref() {
        buffer.read_all(buf)
    } else {
        0
    }
}

pub fn clear_kernel_log() {
    let mut buffer_opt = get_kernel_log_buffer().lock();
    if let Some(buffer) = buffer_opt.as_mut() {
        buffer.clear();
    }
}

pub fn get_kernel_log_unread_size() -> usize {
    let buffer_opt = get_kernel_log_buffer().lock();
    if let Some(buffer) = buffer_opt.as_ref() {
        buffer.unread_size()
    } else {
        0
    }
}

pub fn get_kernel_log_buffer_size() -> usize {
    let buffer_opt = get_kernel_log_buffer().lock();
    if let Some(buffer) = buffer_opt.as_ref() {
        buffer.buffer_size()
    } else {
        KERNEL_LOG_BUFFER_SIZE
    }
}

/// The logger used for Asterinas.
struct AsterLogger;

static LOGGER: AsterLogger = AsterLogger;

impl log::Log for AsterLogger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let timestamp = Jiffies::elapsed().as_duration().as_secs_f64();

        // Always add to kernel log buffer for syslog functionality
        add_to_kernel_log(record.level(), &record.args().to_string());

        // Check console log level - only print to console if message level is high enough
        // (lower numeric values = higher priority)
        let message_level = log_level_to_console_level(record.level());
        let console_level = *CONSOLE_LOG_LEVEL.lock();

        // Only output to console if message priority is high enough (level <= console_level)
        if message_level <= console_level {
            print_logs(record, timestamp);
        }
    }

    fn flush(&self) {}
}

#[cfg(feature = "log_color")]
fn print_logs(record: &Record, timestamp: f64) {
    use owo_colors::Style;

    let timestamp_style = Style::new().green();
    let record_style = Style::new().default_color();
    let level_style = match record.level() {
        log::Level::Error => Style::new().red(),
        log::Level::Warn => Style::new().bright_yellow(),
        log::Level::Info => Style::new().blue(),
        log::Level::Debug => Style::new().bright_green(),
        log::Level::Trace => Style::new().bright_black(),
    };

    super::_print(format_args!(
        "{} {:<5}: {}\n",
        timestamp_style.style(format_args!("[{:>10.3}]", timestamp)),
        level_style.style(record.level()),
        record_style.style(record.args())
    ));
}

#[cfg(not(feature = "log_color"))]
fn print_logs(record: &Record, timestamp: f64) {
    super::_print(format_args!(
        "{} {:<5}: {}\n",
        format_args!("[{:>10.3}]", timestamp),
        record.level(),
        record.args()
    ));
}

pub(super) fn init() {
    ostd::logger::inject_logger(&LOGGER);
}
