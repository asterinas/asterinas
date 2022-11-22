use crate::cell::Cell;
use crate::x86_64_util::out8;
use crate::{IrqAllocateHandle, TrapFrame};
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::{boxed::Box, collections::BinaryHeap};
use core::any::Any;
use lazy_static::lazy_static;
use spin::Mutex;

const MASTER_CMD: u16 = 0x20;
const MASTER_DATA: u16 = MASTER_CMD + 1;
const SLAVE_CMD: u16 = 0xA0;
const SLAVE_DATA: u16 = SLAVE_CMD + 1;

const TIMER_RATE: u32 = 1193182;
/// This value represent the base timer frequency in Hz
pub const TIMER_FREQ: u64 = 100;
const TIMER_PERIOD_IO_PORT: u16 = 0x40;
const TIMER_MODE_IO_PORT: u16 = 0x43;
const TIMER_SQUARE_WAVE: u8 = 0x36;

const TIMER_IRQ_NUM: u8 = 32;

pub static mut TICK: u64 = 0;

lazy_static! {
    static ref TIMER_IRQ: Mutex<IrqAllocateHandle> = Mutex::new(
        crate::trap::allocate_target_irq(TIMER_IRQ_NUM).expect("Timer irq Allocate error")
    );
}

pub fn init() {
    // Start initialization
    out8(MASTER_CMD, 0x11);
    out8(SLAVE_CMD, 0x11);

    // Set offsets
    // map master PIC vector 0x00~0x07 to 0x20~0x27 IRQ number
    out8(MASTER_DATA, 0x20);
    // map slave PIC vector 0x00~0x07 to 0x28~0x2f IRQ number
    out8(SLAVE_DATA, 0x28);

    // Set up cascade, there is slave at IRQ2
    out8(MASTER_DATA, 4);
    out8(SLAVE_DATA, 2);

    // Set up interrupt mode (1 is 8086/88 mode, 2 is auto EOI)
    out8(MASTER_DATA, 1);
    out8(SLAVE_DATA, 1);

    // Unmask timer interrupt
    out8(MASTER_DATA, 0xFE);
    out8(SLAVE_DATA, 0xFF);

    // Ack remaining interrupts
    out8(MASTER_CMD, 0x20);
    out8(SLAVE_CMD, 0x20);

    // Initialize timer.
    let cycle = TIMER_RATE / TIMER_FREQ as u32; // 1ms per interrupt.
    out8(TIMER_MODE_IO_PORT, TIMER_SQUARE_WAVE);
    out8(TIMER_PERIOD_IO_PORT, (cycle & 0xFF) as _);
    out8(TIMER_PERIOD_IO_PORT, (cycle >> 8) as _);
    TIMER_IRQ.lock().on_active(timer_callback);
}

#[inline(always)]
fn ack() {
    out8(MASTER_CMD, 0x20);
}

fn timer_callback(trap_frame: &TrapFrame) {
    // FIXME: disable and enable interupt will cause infinity loop
    // x86_64_util::disable_interrupts();
    ack();
    let current_ms;
    unsafe {
        current_ms = TICK;
        TICK += 1;
    }
    let timeout_list = TIMEOUT_LIST.get();
    let mut callbacks: Vec<Arc<TimerCallback>> = Vec::new();
    while let Some(t) = timeout_list.peek() {
        if t.expire_ms <= current_ms {
            callbacks.push(timeout_list.pop().unwrap());
        } else {
            break;
        }
    }
    for callback in callbacks {
        if callback.is_enable() {
            callback.callback.call((&callback,));
        }
    }
    // x86_64_util::enable_interrupts();
}

lazy_static! {
    static ref TIMEOUT_LIST: Cell<BinaryHeap<Arc<TimerCallback>>> = Cell::new(BinaryHeap::new());
}

pub struct TimerCallback {
    expire_ms: u64,
    data: Arc<dyn Any + Send + Sync>,
    callback: Box<dyn Fn(&TimerCallback) + Send + Sync>,
    enable: Cell<bool>,
}

impl TimerCallback {
    fn new(
        timeout_ms: u64,
        data: Arc<dyn Any + Send + Sync>,
        callback: Box<dyn Fn(&TimerCallback) + Send + Sync>,
    ) -> Self {
        Self {
            expire_ms: timeout_ms,
            data,
            callback,
            enable: Cell::new(true),
        }
    }

    pub fn data(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.data
    }

    /// disable this timeout
    pub fn disable(&self) {
        *self.enable.get() = false;
    }

    /// enable this timeout
    pub fn enable(&self) {
        *self.enable.get() = true;
    }

    pub fn is_enable(&self) -> bool {
        *self.enable
    }
}

impl PartialEq for TimerCallback {
    fn eq(&self, other: &Self) -> bool {
        self.expire_ms == other.expire_ms
    }
}

impl Eq for TimerCallback {}

impl PartialOrd for TimerCallback {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerCallback {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.expire_ms.cmp(&other.expire_ms).reverse()
    }
}

/// add timeout task into timeout list, the frequency see const TIMER_FREQ
///
/// user should ensure that the callback function cannot take too much time
///
pub fn add_timeout_list<F, T>(timeout: u64, data: T, callback: F) -> Arc<TimerCallback>
where
    F: Fn(&TimerCallback) + Send + Sync + 'static,
    T: Any + Send + Sync,
{
    unsafe {
        let timer_callback = TimerCallback::new(TICK + timeout, Arc::new(data), Box::new(callback));
        let arc = Arc::new(timer_callback);
        TIMEOUT_LIST.get().push(arc.clone());
        arc
    }
}
