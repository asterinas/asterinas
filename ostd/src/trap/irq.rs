// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use core::fmt::Debug;

use crate::{
    arch::irq::{self, IrqCallbackHandle, IRQ_ALLOCATOR},
    prelude::*,
    sync::GuardTransfer,
    trap::TrapFrame,
    Error,
};

/// Type alias for the irq callback function.
pub type IrqCallbackFunction = dyn Fn(&TrapFrame) + Sync + Send + 'static;

/// An Interrupt ReQuest(IRQ) line. User can use [`alloc`] or [`alloc_specific`] to get specific IRQ line.
///
/// The IRQ number is guaranteed to be external IRQ number and user can register callback functions to this IRQ resource.
/// When this resource is dropped, all the callback in this will be unregistered automatically.
///
/// [`alloc`]: Self::alloc
/// [`alloc_specific`]: Self::alloc_specific
#[derive(Debug)]
#[must_use]
pub struct IrqLine {
    irq_num: u8,
    #[allow(clippy::redundant_allocation)]
    inner_irq: Arc<&'static irq::IrqLine>,
    callbacks: Vec<IrqCallbackHandle>,
}

impl IrqLine {
    /// Allocates a specific IRQ line.
    pub fn alloc_specific(irq: u8) -> Result<Self> {
        IRQ_ALLOCATOR
            .get()
            .unwrap()
            .lock()
            .alloc_specific(irq as usize)
            .map(|irq_num| Self::new(irq_num as u8))
            .ok_or(Error::NotEnoughResources)
    }

    /// Allocates an available IRQ line.
    pub fn alloc() -> Result<Self> {
        let Some(irq_num) = IRQ_ALLOCATOR.get().unwrap().lock().alloc() else {
            return Err(Error::NotEnoughResources);
        };
        Ok(Self::new(irq_num as u8))
    }

    fn new(irq_num: u8) -> Self {
        // SAFETY: The IRQ number is allocated through `RecycleAllocator`, and it is guaranteed that the
        // IRQ is not one of the important IRQ like cpu exception IRQ.
        Self {
            irq_num,
            inner_irq: unsafe { irq::IrqLine::acquire(irq_num) },
            callbacks: Vec::new(),
        }
    }

    /// Gets the IRQ number.
    pub fn num(&self) -> u8 {
        self.irq_num
    }

    /// Registers a callback that will be invoked when the IRQ is active.
    ///
    /// For each IRQ line, multiple callbacks may be registered.
    pub fn on_active<F>(&mut self, callback: F)
    where
        F: Fn(&TrapFrame) + Sync + Send + 'static,
    {
        self.callbacks.push(self.inner_irq.on_active(callback))
    }

    /// Checks if there are no registered callbacks.
    pub fn is_empty(&self) -> bool {
        self.callbacks.is_empty()
    }

    pub(crate) fn inner_irq(&self) -> &'static irq::IrqLine {
        &self.inner_irq
    }
}

impl Clone for IrqLine {
    fn clone(&self) -> Self {
        Self {
            irq_num: self.irq_num,
            inner_irq: self.inner_irq.clone(),
            callbacks: Vec::new(),
        }
    }
}

impl Drop for IrqLine {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner_irq) == 1 {
            IRQ_ALLOCATOR
                .get()
                .unwrap()
                .lock()
                .free(self.irq_num as usize);
        }
    }
}

/// Disables all IRQs on the current CPU (i.e., locally).
///
/// This function returns a guard object, which will automatically enable local IRQs again when
/// it is dropped. This function works correctly even when it is called in a _nested_ way.
/// The local IRQs shall only be re-enabled when the most outer guard is dropped.
///
/// This function can play nicely with [`SpinLock`] as the type uses this function internally.
/// One can invoke this function even after acquiring a spin lock. And the reversed order is also ok.
///
/// [`SpinLock`]: crate::sync::SpinLock
///
/// # Example
///
/// ```rust
/// use ostd::irq;
///
/// {
///     let _ = irq::disable_local();
///     todo!("do something when irqs are disabled");
/// }
/// ```
pub fn disable_local() -> DisabledLocalIrqGuard {
    DisabledLocalIrqGuard::new()
}

/// A guard for disabled local IRQs.
#[clippy::has_significant_drop]
#[must_use]
pub struct DisabledLocalIrqGuard {
    was_enabled: bool,
}

impl !Send for DisabledLocalIrqGuard {}

impl DisabledLocalIrqGuard {
    fn new() -> Self {
        let was_enabled = irq::is_local_enabled();
        if was_enabled {
            irq::disable_local();
        }
        Self { was_enabled }
    }
}

impl GuardTransfer for DisabledLocalIrqGuard {
    fn transfer_to(&mut self) -> Self {
        let was_enabled = self.was_enabled;
        self.was_enabled = false;
        Self { was_enabled }
    }
}

impl Drop for DisabledLocalIrqGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            irq::enable_local();
        }
    }
}
