// SPDX-License-Identifier: MPL-2.0

//! Run bounded PMU requests on the owner CPU's existing timer path.

use ostd::{
    cpu::{CpuId, PinCurrentCpu},
    cpu_local, irq,
    sync::{LocalIrqDisabled, Waiter},
    task::atomic_mode,
};

use crate::prelude::*;

const MAX_QUEUED_REQUESTS: usize = 32;
const MAX_REQUESTS_PER_TICK: usize = 8;

type Request = Box<dyn FnOnce() + Send>;

cpu_local! {
    static REQUESTS: SpinLock<Option<VecDeque<Request>>, LocalIrqDisabled> =
        SpinLock::new(None);
}

/// Preallocates each CPU's request queue before registering its timer.
pub(super) fn init_on_cpu() {
    let mut requests = VecDeque::new();
    if requests.try_reserve_exact(MAX_QUEUED_REQUESTS).is_err() {
        error!("unable to allocate the remote PMU request queue");
        return;
    }
    let irq_guard = irq::disable_local();
    *REQUESTS.get_on_cpu(irq_guard.current_cpu()).lock() = Some(requests);
}

/// Queues an operation and waits in task context for its own result.
///
/// The operation runs on the target CPU with local IRQs disabled.
/// Release any lock the operation may acquire before calling this function.
/// Returns `EBUSY` if the target CPU's request queue is full.
pub(super) fn call_on_cpu<T: Send + 'static>(
    cpu: CpuId,
    operation: impl FnOnce() -> T + Send + 'static,
) -> Result<T> {
    atomic_mode::might_sleep();
    let (waiter, waker) = Waiter::new_pair();
    let result = Arc::new(SpinLock::<Option<T>, LocalIrqDisabled>::new(None));
    let remote_result = result.clone();
    let request = Box::new(move || {
        let value = operation();
        *remote_result.lock() = Some(value);
        waker.wake_up();
    });

    {
        let mut queue = REQUESTS.get_on_cpu(cpu).lock();
        let requests = queue
            .as_mut()
            .ok_or_else(|| errno!(ENOMEM, "the PMU owner CPU has no request queue"))?;
        if requests.len() == MAX_QUEUED_REQUESTS {
            return Err(errno!(EBUSY, "the PMU owner CPU request queue is full"));
        }
        requests.push_back(request);
    }

    waiter.wait();
    let value = result.lock().take();
    debug_assert!(
        value.is_some(),
        "the PMU request woke before publishing its result"
    );
    value.ok_or_else(|| errno!(EIO, "missing PMU request result"))
}

/// Runs a bounded batch without holding the queue lock across a callback.
pub(super) fn service_requests() {
    let irq_guard = irq::disable_local();
    let requests = REQUESTS.get_on_cpu(irq_guard.current_cpu());
    for _ in 0..MAX_REQUESTS_PER_TICK {
        let request = requests.lock().as_mut().and_then(VecDeque::pop_front);
        let Some(request) = request else { break };
        request();
    }
}
