// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use trapframe::TrapFrame;

use crate::{
    arch::irq::{self, IRQ_LIST, IRQ_NUM_ALLOCATOR},
    prelude::*,
    sync::{Mutex, SpinLock, SpinLockGuard},
    task::{disable_preempt, DisablePreemptGuard},
    util::recycle_allocator::RecycleAllocator,
    Error,
};

pub type IrqCallbackFunction = dyn Fn(&TrapFrame) + Sync + Send + 'static;

/// An Interrupt ReQuest(IRQ) line. User can use `alloc` or `alloc_specific` to get specific IRQ line.
///
/// The IRQ number is guaranteed to be external IRQ number and user can register callback functions to this IRQ resource.
/// When this resrouce is dropped, all the callback in this will be unregistered automatically.
#[derive(Debug)]
#[must_use]
pub struct IrqLine {
    irq_num: u8,
    #[allow(clippy::redundant_allocation)]
    irq: Arc<&'static SystemIrqLine>,
    callbacks: Vec<IrqCallbackHandle>,
}

impl IrqLine {
    pub fn alloc_specific(irq: u8) -> Result<Self> {
        if IRQ_NUM_ALLOCATOR.lock().get_target(irq as usize) {
            Ok(Self::new(irq))
        } else {
            Err(Error::NotEnoughResources)
        }
    }

    pub fn alloc() -> Result<Self> {
        let irq_num = IRQ_NUM_ALLOCATOR.lock().alloc();
        if irq_num == usize::MAX {
            Err(Error::NotEnoughResources)
        } else {
            Ok(Self::new(irq_num as u8))
        }
    }

    fn new(irq_num: u8) -> Self {
        // Safety: The IRQ number is allocated through `RecycleAllocator`, and it is guaranteed that the
        // IRQ is not one of the important IRQ like cpu exception IRQ.
        Self {
            irq_num,
            irq: unsafe { SystemIrqLine::acquire(irq_num) },
            callbacks: Vec::new(),
        }
    }

    /// Get the IRQ number.
    pub fn num(&self) -> u8 {
        self.irq_num
    }

    /// Register a callback that will be invoked when the IRQ is active.
    ///
    /// For each IRQ line, multiple callbacks may be registered.
    pub fn on_active<F>(&mut self, callback: F)
    where
        F: Fn(&TrapFrame) + Sync + Send + 'static,
    {
        self.callbacks.push(self.irq.on_active(callback))
    }

    pub fn is_empty(&self) -> bool {
        self.callbacks.is_empty()
    }
}

impl Clone for IrqLine {
    fn clone(&self) -> Self {
        Self {
            irq_num: self.irq_num,
            irq: self.irq.clone(),
            callbacks: Vec::new(),
        }
    }
}

impl Drop for IrqLine {
    fn drop(&mut self) {
        if Arc::strong_count(&self.irq) == 1 {
            IRQ_NUM_ALLOCATOR.lock().dealloc(self.irq_num as usize);
        }
    }
}

/// Disable all IRQs on the current CPU (i.e., locally).
///
/// This function returns a guard object, which will automatically enable local IRQs again when
/// it is dropped. This function works correctly even when it is called in a _nested_ way.
/// The local IRQs shall only be re-enabled when the most outer guard is dropped.
///
/// This function can play nicely with `SpinLock` as the type uses this function internally.
/// One can invoke this function even after acquiring a spin lock. And the reversed order is also ok.
///
/// # Example
///
/// ```rust
/// use aster_frame::irq;
///
/// {
///     let _ = irq::disable_local();
///     todo!("do something when irqs are disabled");
/// }
/// ```
#[must_use]
pub fn disable_local() -> DisabledLocalIrqGuard {
    DisabledLocalIrqGuard::new()
}

/// A guard for disabled local IRQs.
pub struct DisabledLocalIrqGuard {
    was_enabled: bool,
    preempt_guard: DisablePreemptGuard,
}

impl !Send for DisabledLocalIrqGuard {}

impl DisabledLocalIrqGuard {
    fn new() -> Self {
        let was_enabled = irq::is_local_enabled();
        if was_enabled {
            irq::disable_local();
        }
        let preempt_guard = disable_preempt();
        Self {
            was_enabled,
            preempt_guard,
        }
    }

    /// Transfer the saved IRQ status of this guard to a new guard.
    /// The saved IRQ status of this guard is cleared.
    pub fn transfer_to(&mut self) -> Self {
        let was_enabled = self.was_enabled;
        self.was_enabled = false;
        Self {
            was_enabled,
            preempt_guard: disable_preempt(),
        }
    }
}

impl Drop for DisabledLocalIrqGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            irq::enable_local();
        }
    }
}

/// A global exclusive IRQ line in the system. The architecture specific code should initialize
/// the IRQ line collection in the system.
#[derive(Debug)]
pub(crate) struct SystemIrqLine {
    pub(crate) irq_num: u8,
    pub(crate) callback_list: SpinLock<Vec<CallbackElement>>,
}

impl SystemIrqLine {
    /// Acquire an interrupt request line.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as manipulating interrupt lines is
    /// considered a dangerous operation.
    #[allow(clippy::redundant_allocation)]
    pub unsafe fn acquire(irq_num: u8) -> Arc<&'static Self> {
        Arc::new(IRQ_LIST.get().unwrap().get(irq_num as usize).unwrap())
    }

    /// Get the IRQ number.
    pub fn num(&self) -> u8 {
        self.irq_num
    }

    pub fn callback_list(&self) -> SpinLockGuard<'_, alloc::vec::Vec<CallbackElement>> {
        self.callback_list.lock()
    }

    /// Register a callback that will be invoked when the IRQ is active.
    ///
    /// A handle to the callback is returned. Dropping the handle
    /// automatically unregisters the callback.
    ///
    /// For each IRQ line, multiple callbacks may be registered.
    pub fn on_active<F>(&self, callback: F) -> IrqCallbackHandle
    where
        F: Fn(&TrapFrame) + Sync + Send + 'static,
    {
        let allocate_id = CALLBACK_ID_ALLOCATOR.lock().alloc();
        self.callback_list.lock().push(CallbackElement {
            function: Box::new(callback),
            id: allocate_id,
        });
        IrqCallbackHandle {
            irq_num: self.irq_num,
            id: allocate_id,
        }
    }
}

/// The handle to a registered callback for a IRQ line.
///
/// When the handle is dropped, the callback will be unregistered automatically.
#[must_use]
#[derive(Debug)]
pub(crate) struct IrqCallbackHandle {
    irq_num: u8,
    id: usize,
}

impl Drop for IrqCallbackHandle {
    fn drop(&mut self) {
        let mut a = IRQ_LIST
            .get()
            .unwrap()
            .get(self.irq_num as usize)
            .unwrap()
            .callback_list
            .lock();
        a.retain(|item| item.id != self.id);
        CALLBACK_ID_ALLOCATOR.lock().dealloc(self.id);
    }
}

static CALLBACK_ID_ALLOCATOR: Mutex<RecycleAllocator> = Mutex::new(RecycleAllocator::new());
pub(crate) struct CallbackElement {
    function: Box<dyn Fn(&TrapFrame) + Send + Sync + 'static>,
    id: usize,
}

impl CallbackElement {
    pub fn call(&self, element: &TrapFrame) {
        self.function.call((element,));
    }
}

impl Debug for CallbackElement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CallbackElement")
            .field("id", &self.id)
            .finish()
    }
}
