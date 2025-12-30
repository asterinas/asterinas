use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use alloc::sync::Arc;

use log::{Level, LevelFilter, Record};
use ring_buffer::RingBuffer;
use ostd::sync::SpinLock;
use ostd::mm::VmIo;

const LOG_BUFFER_CAPACITY: usize = 64 * 1024;
const FORMAT_BUF_CAPACITY: usize = 512;
const COPY_CHUNK: usize = 512;

static KLOG: SpinLock<Option<Arc<KernelLog>>> = SpinLock::new(None);

pub fn init_klog() {
    let _ = klog();
}

fn klog() -> Arc<KernelLog> {
    let mut guard = KLOG.lock();
    if guard.is_none() {
        *guard = Some(Arc::new(KernelLog::new()));
    }
    guard.as_ref().unwrap().clone()
}

pub fn append_log(record: &Record, timestamp: &Duration) {
    let mut scratch = [0u8; FORMAT_BUF_CAPACITY];
    let mut writer = FixedBuf::new(&mut scratch);

    let secs = timestamp.as_secs();
    let millis = timestamp.subsec_millis();
    let _ = write!(
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

pub fn should_print(level: Level) -> bool {
    klog().should_print(level)
}

pub fn console_level() -> LevelFilter {
    klog().console_level()
}

pub fn console_set_level(level: LevelFilter) -> LevelFilter {
    klog().set_console_level(level, false)
}

pub fn console_off() -> LevelFilter {
    klog().set_console_level(LevelFilter::Off, true)
}

pub fn console_on() -> LevelFilter {
    klog().restore_console_level()
}

pub fn klog_read(dst: &mut [u8]) -> usize {
    klog().read(dst)
}

pub fn klog_read_all(dst: &mut [u8], offset: usize, window_len: usize) -> usize {
    klog().read_all(dst, offset, window_len)
}

pub fn mark_clear() {
    klog().mark_clear();
}

pub fn klog_size_unread() -> usize {
    klog().size_unread()
}

pub fn klog_capacity() -> usize {
    LOG_BUFFER_CAPACITY
}

pub fn read_all_requires_cap() -> bool {
    klog().dmesg_restrict()
}

struct KernelLog {
    buffer: SpinLock<RingBuffer<u8>>,
    clear_tail: SpinLock<usize>,
    dmesg_restrict: AtomicBool,
    console_level: SpinLock<LevelFilter>,
    saved_console_level: SpinLock<Option<LevelFilter>>,
}

impl KernelLog {
    fn new() -> Self {
        // klog records whatever records reach the logger; global filtering is configured elsewhere.
        Self {
            buffer: SpinLock::new(RingBuffer::new(LOG_BUFFER_CAPACITY)),
            clear_tail: SpinLock::new(0),
            dmesg_restrict: AtomicBool::new(false),
            console_level: SpinLock::new(LevelFilter::Info),
            saved_console_level: SpinLock::new(None),
        }
    }

    fn append(&self, mut bytes: &[u8]) {
        let mut buf = self.buffer.lock();
        let cap = buf.capacity();

        if bytes.len() > cap {
            bytes = &bytes[bytes.len() - cap..];
            buf.reset_head();
        }

        // Drop oldest data if needed.
        let free = buf.free_len();
        let need_drop = bytes.len().saturating_sub(free);
        if need_drop > 0 {
            let head = buf.head();
            buf.advance_head(head, need_drop);
            self.bump_clear_tail(&mut buf);
        }

        buf.push_slice(bytes).expect("push_slice must succeed after drop");
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
                let take = core::cmp::min(
                    core::cmp::min(dst.len() - copied, COPY_CHUNK),
                    available,
                );
                copy_from(&buf, buf.head().0, &mut dst[copied..copied + take]);
                let head = buf.head();
                buf.advance_head(head, take);
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

    fn set_console_level(&self, level: LevelFilter, save_old: bool) -> LevelFilter {
        let mut saved = self.saved_console_level.lock();
        let mut current = self.console_level.lock();
        if save_old && saved.is_none() {
            *saved = Some(*current);
        }
        let old = *current;
        *current = level;
        old
    }

    fn restore_console_level(&self) -> LevelFilter {
        let mut saved = self.saved_console_level.lock();
        let mut current = self.console_level.lock();
        if let Some(prev) = saved.take() {
            *current = prev;
        }
        *current
    }

    fn console_level(&self) -> LevelFilter {
        *self.console_level.lock()
    }

    fn should_print(&self, level: Level) -> bool {
        match *self.console_level.lock() {
            LevelFilter::Off => false,
            LevelFilter::Error => matches!(level, Level::Error),
            LevelFilter::Warn => matches!(level, Level::Error | Level::Warn),
            LevelFilter::Info => matches!(level, Level::Error | Level::Warn | Level::Info),
            LevelFilter::Debug => !matches!(level, Level::Trace),
            LevelFilter::Trace => true,
        }
    }
}

fn copy_from(rb: &RingBuffer<u8>, start: usize, dst: &mut [u8]) {
    let cap = rb.capacity();
    let offset = start & (cap - 1);
    if offset + dst.len() > cap {
        let first = cap - offset;
        rb.segment()
            .read_slice(offset, &mut dst[..first])
            .unwrap();
        rb.segment()
            .read_slice(0, &mut dst[first..])
            .unwrap();
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

