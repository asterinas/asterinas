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

fn sync_current() {
    remote::service_requests();
    if activity::has_events()
        && let Some(perf) = Thread::current().as_deref().and_then(Thread::perf)
    {
        perf.on_tick();
    }
}

pub(crate) fn to_error(error: PmuError) -> Error {
    let (errno, message) = match error {
        PmuError::Unsupported => (Errno::EOPNOTSUPP, "no supported hardware PMU"),
        PmuError::UnsupportedEvent => (Errno::EOPNOTSUPP, "PMU event is unavailable"),
        PmuError::NoCounter => (Errno::EBUSY, "PMU counter resource is occupied"),
        PmuError::Overflow => (
            Errno::EOVERFLOW,
            "PMU count or observation interval overflow",
        ),
        PmuError::WrongCpu | PmuError::InvalidHandle | PmuError::HardwareFault => {
            (Errno::EIO, "PMU register or ownership operation failed")
        }
    };
    Error::with_message(errno, message)
}
