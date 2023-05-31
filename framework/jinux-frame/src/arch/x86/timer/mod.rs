pub mod apic;
pub mod hpet;
pub mod pit;

use core::any::Any;
use core::sync::atomic::{AtomicU64, Ordering};

use alloc::{boxed::Box, collections::BinaryHeap, sync::Arc, vec::Vec};
use spin::{Mutex, Once};
use trapframe::TrapFrame;

use crate::arch::x86::kernel;
use crate::trap::IrqAllocateHandle;

pub const TIMER_IRQ_NUM: u8 = 32;
pub static TICK: AtomicU64 = AtomicU64::new(0);

static TIMER_IRQ: Once<IrqAllocateHandle> = Once::new();

pub fn init() {
    TIMEOUT_LIST.call_once(|| Mutex::new(BinaryHeap::new()));
    if kernel::xapic::has_apic() {
        apic::init();
    } else {
        pit::init();
    }
    let mut timer_irq =
        crate::trap::allocate_target_irq(TIMER_IRQ_NUM).expect("Timer irq Allocate error");
    timer_irq.on_active(timer_callback);
    TIMER_IRQ.call_once(|| timer_irq);
}

fn timer_callback(trap_frame: &TrapFrame) {
    let current_ms = TICK.fetch_add(1, Ordering::SeqCst);
    let mut timeout_list = TIMEOUT_LIST.get().unwrap().lock();
    let mut callbacks: Vec<Arc<TimerCallback>> = Vec::new();
    while let Some(t) = timeout_list.peek() {
        if t.expire_ms <= current_ms && t.is_enable() {
            callbacks.push(timeout_list.pop().unwrap());
        } else {
            break;
        }
    }
    drop(timeout_list);
    for callback in callbacks {
        callback.callback.call((&callback,));
    }
}

static TIMEOUT_LIST: Once<Mutex<BinaryHeap<Arc<TimerCallback>>>> = Once::new();

pub struct TimerCallback {
    expire_ms: u64,
    data: Arc<dyn Any + Send + Sync>,
    callback: Box<dyn Fn(&TimerCallback) + Send + Sync>,
    enable: Mutex<bool>,
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
            enable: Mutex::new(true),
        }
    }

    pub fn data(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.data
    }

    /// disable this timeout
    pub fn disable(&self) {
        *self.enable.lock() = false;
    }

    /// enable this timeout
    pub fn enable(&self) {
        *self.enable.lock() = true;
    }

    pub fn is_enable(&self) -> bool {
        *self.enable.lock()
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
    let timer_callback = TimerCallback::new(
        TICK.load(Ordering::SeqCst) + timeout,
        Arc::new(data),
        Box::new(callback),
    );
    let arc = Arc::new(timer_callback);
    TIMEOUT_LIST.get().unwrap().lock().push(arc.clone());
    arc
}

/// The time since the system boots up.
/// The currently returned results are in milliseconds.
pub fn read_monotonic_milli_seconds() -> u64 {
    TICK.load(Ordering::SeqCst)
}
