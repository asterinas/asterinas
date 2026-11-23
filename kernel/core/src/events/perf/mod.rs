// SPDX-License-Identifier: MPL-2.0

//! Per-thread hardware performance counting backed by OSTD's CPU-local PMU.

mod activity;
mod context;
mod credentials;
mod enabled_time;
mod observation;
mod record;
mod remote;

use ostd::pmu::{self, PmuError};

use self::context::{Operation, Snapshot};
pub(crate) use self::context::{EventSpec, PerfContext};
use crate::{prelude::*, thread::Thread};

/// Probes the CPU-local PMU before its timer callbacks begin running.
pub(crate) fn init_on_cpu() {
    if !pmu::HAS_BACKEND {
        return;
    }
    if let Err(error) = pmu::init_on_cpu() {
        debug!("CPU-local PMU initialization skipped: {error:?}");
    }
    remote::init_on_cpu();
    ostd::timer::register_callback_on_cpu(sync_current);
}

pub(crate) fn to_error(error: PmuError) -> Error {
    match error {
        PmuError::Unsupported => errno!(EOPNOTSUPP, "no supported hardware PMU"),
        PmuError::UnsupportedEvent => errno!(EOPNOTSUPP, "PMU event is unavailable"),
        PmuError::NoCounter => errno!(EBUSY, "PMU counter resource is occupied"),
        PmuError::Overflow => errno!(EOVERFLOW, "PMU count or observation interval overflow"),
        PmuError::WrongCpu | PmuError::InvalidHandle | PmuError::HardwareFault => {
            errno!(EIO, "PMU register or ownership operation failed")
        }
    }
}

fn sync_current() {
    remote::service_requests();
    if activity::has_events()
        && let Some(perf) = Thread::current().as_deref().and_then(Thread::perf)
    {
        perf.on_tick();
    }
}
