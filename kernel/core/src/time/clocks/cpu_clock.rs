// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::time::Duration;

use ostd::{
    sync::{LocalIrqDisabled, SpinLock},
    timer::Jiffies,
};

use crate::time::Clock;

/// A clock used to record the CPU time for processes and threads.
pub struct CpuClock {
    time: SpinLock<Jiffies, LocalIrqDisabled>,
}

/// A profiling clock that contains a user CPU clock and a kernel CPU clock.
///
/// These two clocks record the CPU time in user mode and kernel mode respectively.
/// Reading this clock directly returns the sum of both times.
pub struct ProfClock {
    user_clock: Arc<CpuClock>,
    kernel_clock: Arc<CpuClock>,
}

impl CpuClock {
    /// Creates a new `CpuClock`. The recorded time is initialized to 0.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            time: SpinLock::new(Jiffies::new(0)),
        })
    }

    /// Adds `jiffies` to the original recorded time to update the `CpuClock`.
    pub fn add_jiffies(&self, jiffies: u64) {
        self.time.lock().add(jiffies);
    }

    /// Reads the current time of this clock in [`Jiffies`].
    pub fn read_jiffies(&self) -> Jiffies {
        *self.time.lock()
    }
}

impl Clock for CpuClock {
    fn read_time(&self) -> Duration {
        self.read_jiffies().as_duration()
    }
}

impl ProfClock {
    /// Creates a new `ProfClock`. The recorded time is initialized to 0.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            user_clock: CpuClock::new(),
            kernel_clock: CpuClock::new(),
        })
    }

    /// Returns a reference to the user CPU clock in this profiling clock.
    pub fn user_clock(&self) -> &Arc<CpuClock> {
        &self.user_clock
    }

    /// Returns a reference to the kernel CPU clock in this profiling clock.
    pub fn kernel_clock(&self) -> &Arc<CpuClock> {
        &self.kernel_clock
    }
}

impl Clock for ProfClock {
    fn read_time(&self) -> Duration {
        self.user_clock.read_time() + self.kernel_clock.read_time()
    }
}
