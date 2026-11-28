// SPDX-License-Identifier: MPL-2.0

//! Shared PMU configuration and error types.

/// A hardware performance event to count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Event {
    /// Processor clock cycles.
    CpuCycles,
    /// Retired architectural instructions.
    Instructions,
}

/// The execution privilege levels included in a hardware event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountingScope {
    /// User-mode execution only.
    User,
    /// Kernel-mode execution only.
    Kernel,
    /// Both user-mode and kernel-mode execution.
    UserAndKernel,
}

/// A hardware event and its requested privilege-level filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventConfig {
    event: Event,
    scope: CountingScope,
}

impl EventConfig {
    /// Creates a new event configuration.
    pub const fn new(event: Event, scope: CountingScope) -> Self {
        Self { event, scope }
    }

    /// Returns the hardware event to measure.
    pub const fn event(&self) -> Event {
        self.event
    }

    /// Returns whether execution in user mode is counted.
    pub const fn is_user_counted(&self) -> bool {
        matches!(
            self.scope,
            CountingScope::User | CountingScope::UserAndKernel
        )
    }

    /// Returns whether execution in kernel mode is counted.
    pub const fn is_kernel_counted(&self) -> bool {
        matches!(
            self.scope,
            CountingScope::Kernel | CountingScope::UserAndKernel
        )
    }
}

/// The detected hardware PMU interface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendKind {
    /// Intel architectural PMU (fixed and general-purpose counters).
    IntelArchitectural,
    /// AMD core performance counters.
    AmdCore,
}

/// The PMU capabilities cached for one CPU during initialization.
#[derive(Clone, Copy, Debug)]
pub struct PmuCaps {
    pub(crate) backend: BackendKind,
    pub(crate) counters: usize,
    pub(crate) width_bits: u8,
    pub(crate) supports_cycles: bool,
    pub(crate) supports_instructions: bool,
}

impl PmuCaps {
    /// Returns the detected hardware PMU interface.
    pub const fn backend(&self) -> BackendKind {
        self.backend
    }

    /// Returns the number of detected hardware counters on this CPU.
    pub const fn counters(&self) -> usize {
        self.counters
    }

    /// Reports whether the backend has a counter mapping for this event.
    ///
    /// This describes the hardware capability before temporary allocations.
    /// Configuring a supported event can still fail if its counters are busy.
    pub const fn supports_event(&self, event: Event) -> bool {
        match event {
            Event::CpuCycles => self.supports_cycles,
            Event::Instructions => self.supports_instructions,
        }
    }

    /// Returns the minimum bit width of the detected hardware counters.
    /// Use [`super::counter_width`] for a particular allocation.
    pub const fn width_bits(&self) -> u8 {
        self.width_bits
    }
}

/// A failure returned by a PMU operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PmuError {
    /// No supported PMU is exposed on this CPU.
    Unsupported,
    /// The requested event has no supported hardware mapping.
    UnsupportedEvent,
    /// No managed counter resource is free for the event.
    NoCounter,
    /// A handle was accessed from a CPU other than its owner.
    WrongCpu,
    /// The counter allocation has ended or is in an invalid state.
    InvalidHandle,
    /// The CPU reported inconsistent PMU capabilities.
    HardwareFault,
    /// A counter accumulator overflowed or the observation interval was too long.
    Overflow,
}

/// A specialized result type for PMU operations.
pub type Result<T> = core::result::Result<T, PmuError>;
