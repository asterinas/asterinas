pub mod apic;
pub mod hpet;
pub mod pit;

use core::any::Any;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use alloc::{boxed::Box, collections::BinaryHeap, sync::Arc, vec::Vec};
use spin::Once;
use trapframe::TrapFrame;

use crate::arch::x86::kernel;
use crate::config::TIMER_FREQ;
use crate::sync::SpinLock;
use crate::trap::IrqLine;

use self::apic::APIC_TIMER_CALLBACK;

pub static TIMER_IRQ_NUM: AtomicU8 = AtomicU8::new(32);
pub static TICK: AtomicU64 = AtomicU64::new(0);

static TIMER_IRQ: Once<IrqLine> = Once::new();

pub fn init() {
    TIMEOUT_LIST.call_once(|| SpinLock::new(BinaryHeap::new()));
    if kernel::apic::APIC_INSTANCE.is_completed() {
        // Get the free irq number first. Use `allocate_target_irq` to get the Irq handle after dropping it.
        // Because the function inside `apic::init` will allocate this irq.
        let irq = IrqLine::alloc().unwrap();
        TIMER_IRQ_NUM.store(irq.num(), Ordering::Relaxed);
        drop(irq);
        apic::init();
    } else {
        pit::init();
    };
    let mut timer_irq = IrqLine::alloc_specific(TIMER_IRQ_NUM.load(Ordering::Relaxed)).unwrap();
    timer_irq.on_active(timer_callback);
    TIMER_IRQ.call_once(|| timer_irq);
}

fn timer_callback(trap_frame: &TrapFrame) {
    let current_ticks = TICK.fetch_add(1, Ordering::SeqCst);

    let callbacks = {
        let mut callbacks = Vec::new();
        let mut timeout_list = TIMEOUT_LIST.get().unwrap().lock_irq_disabled();

        while let Some(t) = timeout_list.peek() {
            if t.is_cancelled() {
                // Just ignore the cancelled callback
                timeout_list.pop();
            } else if t.expire_ticks <= current_ticks {
                callbacks.push(timeout_list.pop().unwrap());
            } else {
                break;
            }
        }
        callbacks
    };

    for callback in callbacks {
        (callback.callback)(&callback);
    }

    if APIC_TIMER_CALLBACK.is_completed() {
        APIC_TIMER_CALLBACK.get().unwrap().call(());
    }
}

static TIMEOUT_LIST: Once<SpinLock<BinaryHeap<Arc<TimerCallback>>>> = Once::new();

pub struct TimerCallback {
    expire_ticks: u64,
    data: Arc<dyn Any + Send + Sync>,
    callback: Box<dyn Fn(&TimerCallback) + Send + Sync>,
    is_cancelled: AtomicBool,
}

impl TimerCallback {
    fn new(
        timeout_ticks: u64,
        data: Arc<dyn Any + Send + Sync>,
        callback: Box<dyn Fn(&TimerCallback) + Send + Sync>,
    ) -> Self {
        Self {
            expire_ticks: timeout_ticks,
            data,
            callback,
            is_cancelled: AtomicBool::new(false),
        }
    }

    pub fn data(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.data
    }

    /// Whether the set timeout is reached
    pub fn is_expired(&self) -> bool {
        let current_tick = TICK.load(Ordering::Acquire);
        self.expire_ticks <= current_tick
    }

    /// Cancel a timer callback. If the callback function has not been called,
    /// it will never be called again.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Release);
    }

    // Whether the timer callback is cancelled.
    fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Acquire)
    }
}

impl PartialEq for TimerCallback {
    fn eq(&self, other: &Self) -> bool {
        self.expire_ticks == other.expire_ticks
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
        self.expire_ticks.cmp(&other.expire_ticks).reverse()
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
        TICK.load(Ordering::Acquire) + timeout,
        Arc::new(data),
        Box::new(callback),
    );
    let arc = Arc::new(timer_callback);
    TIMEOUT_LIST
        .get()
        .unwrap()
        .lock_irq_disabled()
        .push(arc.clone());
    arc
}

/// The time since the system boots up.
/// The currently returned results are in milliseconds.
pub fn read_monotonic_milli_seconds() -> u64 {
    TICK.load(Ordering::Acquire) * (1000 / TIMER_FREQ)
}
