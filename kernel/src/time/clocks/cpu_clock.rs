// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::time::Duration;

use ostd::sync::SpinLock;

use crate::time::Clock;

/// A clock used to record the CPU time for processes and threads.
pub struct CpuClock {
    time: SpinLock<Duration>,
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
            time: SpinLock::new(Duration::ZERO),
        })
    }

    /// Adds `interval` to the original recorded time to update the `CpuClock`.
    pub fn add_time(&self, interval: Duration) {
        *self.time.disable_irq().lock() += interval;
    }
}

impl Clock for CpuClock {
    fn read_time(&self) -> Duration {
        *self.time.disable_irq().lock()
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
