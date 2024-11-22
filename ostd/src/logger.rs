// SPDX-License-Identifier: MPL-2.0

//! Logging support.
//!
//! Currently the logger writes the logs into the [`LogBuffer`].
//!
//! This module guarantees _atomicity_ under concurrency: messages are always
//! written in their entirety without being mixed with messages generated
//! concurrently on other cores.
//!
//! IRQs are disabled while writing. So do not log long log messages.

use core::{
    cmp::min,
    fmt::{self, Write},
    str::FromStr,
    time::Duration,
};

use log::{Level, LevelFilter, Metadata, Record};

use crate::{
    boot::{kcmdline::ModuleArg, kernel_cmdline},
    sync::SpinLock,
    timer::Jiffies,
};

/// The size of the text parts of `LogBuffer`. The maximum number of bytes that can be
/// written to the text part of the `LogBuffer` equals to `BUFFER_SIZE` - 1
/// (used to distinguish whether it is empty).
const BUFFER_SIZE: usize = 64 * 1024;
/// The maximum number of logs that the `LogBuffer` can accommodate.
const MAX_LOG_NUM: usize = 4 * 1024;

/// `SplitStr` represents a string slice stored in a ring buffer.
///
/// Here `additional_text` is used to store the latter part of the string
/// slice in case of a split at the buffer tail.
pub struct SplitStr<'a> {
    text: &'a str,
    additional_text: &'a str,
}

impl fmt::Display for SplitStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.text, self.additional_text)
    }
}

impl<'a> SplitStr<'a> {
    fn from_buffer_at(buffer: &'a [u8], begin_index: usize, end_index: usize) -> Self {
        debug_assert!(begin_index < buffer.len());
        debug_assert!(end_index < buffer.len());

        if end_index >= begin_index {
            Self {
                text: unsafe { core::str::from_utf8_unchecked(&buffer[begin_index..end_index]) },
                additional_text: "",
            }
        } else {
            let text = unsafe { core::str::from_utf8_unchecked(&buffer[begin_index..]) };
            let additional_text = unsafe { core::str::from_utf8_unchecked(&buffer[..end_index]) };
            Self {
                text,
                additional_text,
            }
        }
    }
}

/// A log record read from a [`LogBuffer`].
pub struct LogRecord<'a> {
    module: SplitStr<'a>,
    context: SplitStr<'a>,
    log_meta: &'a LogMeta,
}

impl LogRecord<'_> {
    /// Returns the log level of this log record.
    pub fn level(&self) -> &Level {
        &self.log_meta.level
    }

    /// Returns the timestamp when the log was generated.
    pub fn timestamp(&self) -> &Duration {
        &self.log_meta.timestamp
    }

    /// Returns the module where the log was generated.
    pub fn module(&self) -> &SplitStr<'_> {
        &self.module
    }

    /// Returns the context of this log.
    pub fn context(&self) -> &SplitStr<'_> {
        &self.context
    }
}

#[derive(Debug, Clone, Copy)]
struct LogMeta {
    context_len: usize,
    module_len: usize,
    level: Level,
    timestamp: Duration,
}

impl LogMeta {
    const fn zero() -> Self {
        Self {
            context_len: 0,
            module_len: 0,
            level: Level::Error,
            timestamp: Duration::ZERO,
        }
    }
}

/// A buffer for writing log content.
///
/// This buffer consists of two ring buffers, `text_buffer` and `meta_buffer`,
/// which record variable-length information and fixed-length attributes of log operations
/// respectively. When either buffer is full and a new log operation requires overwriting,
/// the log being overwritten will be removed from both ring buffers.
pub struct LogBuffer {
    text_buf: [u8; BUFFER_SIZE],
    text_read_index: usize,
    text_write_index: usize,
    meta_buf: [LogMeta; MAX_LOG_NUM],
    meta_read_index: usize,
    meta_write_index: usize,
}

impl LogBuffer {
    const fn new() -> Self {
        Self {
            text_buf: [0; BUFFER_SIZE],
            text_read_index: 0,
            text_write_index: 0,
            meta_buf: [LogMeta::zero(); MAX_LOG_NUM],
            meta_read_index: 0,
            meta_write_index: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.text_read_index == self.text_write_index
    }

    fn advance_read_index_for_overwrite(&mut self) -> usize {
        if self.is_empty() {
            return 0;
        }

        let read_meta = &self.meta_buf[self.meta_read_index];
        let offset = read_meta.context_len + read_meta.module_len;
        self.text_read_index = (self.text_read_index + offset) % BUFFER_SIZE;
        self.meta_read_index = (self.meta_read_index + 1) % MAX_LOG_NUM;

        offset
    }

    fn write_log(&mut self, record: &Record, timestamp: Duration) -> fmt::Result {
        if self.meta_read_index == self.meta_write_index {
            self.advance_read_index_for_overwrite();
        }

        let module_len = if let Some(module_path) = record.module_path() {
            self.write_str(module_path)?;
            module_path.len()
        } else {
            0
        };

        let context_len = {
            let old_index = self.text_write_index;
            self.write_fmt(format_args!("{}", record.args()))?;
            (self.text_write_index + BUFFER_SIZE - old_index) % BUFFER_SIZE
        };

        let new_meta = LogMeta {
            level: record.level(),
            timestamp,
            context_len,
            module_len,
        };
        self.meta_buf[self.meta_write_index] = new_meta;
        self.meta_write_index = (self.meta_write_index + 1) % MAX_LOG_NUM;
        Ok(())
    }

    /// Reads a recorded log from the buffer.
    ///
    /// If the buffer is empty, returns `None`.
    pub fn read_log(&mut self) -> Option<LogRecord> {
        if self.is_empty() {
            return None;
        }

        let log_meta = &self.meta_buf[self.meta_read_index];

        let module_end_index = (self.text_read_index + log_meta.module_len) % BUFFER_SIZE;
        let module =
            SplitStr::from_buffer_at(&self.text_buf, self.text_read_index, module_end_index);

        let context_end_index = (module_end_index + log_meta.context_len) % BUFFER_SIZE;
        let context = SplitStr::from_buffer_at(&self.text_buf, module_end_index, context_end_index);

        self.meta_read_index = (self.meta_read_index + 1) % MAX_LOG_NUM;
        self.text_read_index = context_end_index;

        Some(LogRecord {
            module,
            context,
            log_meta,
        })
    }
}

impl fmt::Write for LogBuffer {
    fn write_str(&mut self, input: &str) -> fmt::Result {
        let mut remaining_space = if self.text_write_index >= self.text_read_index {
            BUFFER_SIZE - self.text_write_index + self.text_read_index - 1
        } else {
            self.text_read_index - self.text_write_index - 1
        };

        while input.len() > remaining_space {
            let offset = self.advance_read_index_for_overwrite();
            if offset == 0 {
                break;
            }

            remaining_space += offset;
        }

        let end_index = self.text_write_index + input.len();
        let input_slice = input.as_bytes();
        if end_index <= BUFFER_SIZE {
            self.text_buf[self.text_write_index..end_index].copy_from_slice(input_slice);
        } else {
            let first_part_len = BUFFER_SIZE - self.text_write_index;
            let second_part_len = min(BUFFER_SIZE, input.len()) - first_part_len;

            self.text_buf[self.text_write_index..].copy_from_slice(&input_slice[..first_part_len]);
            self.text_buf[..second_part_len].copy_from_slice(&input_slice[first_part_len..]);
        }

        self.text_write_index = end_index % BUFFER_SIZE;
        Ok(())
    }
}

struct Logger {
    pub buffer: SpinLock<LogBuffer>,
}

impl Logger {
    const fn new() -> Self {
        Self {
            buffer: SpinLock::new(LogBuffer::new()),
        }
    }
}

static LOGGER: Logger = Logger::new();

impl log::Log for Logger {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let timestamp: Duration = Jiffies::elapsed().as_duration();

        let mut buffer = self.buffer.disable_irq().lock();
        buffer.write_log(record, timestamp).unwrap();
    }

    fn flush(&self) {}
}

/// Initialize the logger. Users should avoid using the log macros before this function is called.
pub(crate) fn init() {
    let level = get_log_level().unwrap_or(LevelFilter::Off);
    log::set_max_level(level);
    log::set_logger(&LOGGER).unwrap();
}

fn get_log_level() -> Option<LevelFilter> {
    let module_args = kernel_cmdline().get_module_args("ostd")?;

    let value = {
        let value = module_args.iter().find_map(|arg| match arg {
            ModuleArg::KeyVal(name, value) if name.as_bytes() == "log_level".as_bytes() => {
                Some(value)
            }
            _ => None,
        })?;
        value.as_c_str().to_str().ok()?
    };
    LevelFilter::from_str(value).ok()
}

/// Consumes the contents of the log buffer.
///
/// Users can process the content in the log buffer using their preferred
/// methods by passing in a closure.
pub fn consume_buffer_with<F>(f: F)
where
    F: FnOnce(&mut LogBuffer),
{
    let mut buffer = LOGGER.buffer.disable_irq().lock();
    if buffer.is_empty() {
        return;
    }
    f(&mut buffer)
}
