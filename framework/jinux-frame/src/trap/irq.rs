use crate::arch::irq;
use crate::arch::irq::{IRQ_LIST, NOT_USING_IRQ};
use crate::cpu::CpuLocal;
use crate::cpu_local;
use crate::util::recycle_allocator::RecycleAllocator;
use crate::{prelude::*, Error};

use core::fmt::Debug;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use spin::{Mutex, MutexGuard};
use trapframe::TrapFrame;

pub fn allocate_irq() -> Result<IrqAllocateHandle> {
    let irq_num = NOT_USING_IRQ.lock().alloc();
    if irq_num == usize::MAX {
        Err(Error::NotEnoughResources)
    } else {
        Ok(IrqAllocateHandle::new(irq_num as u8))
    }
}

pub(crate) fn allocate_target_irq(target_irq: u8) -> Result<IrqAllocateHandle> {
    if NOT_USING_IRQ.lock().get_target(target_irq as usize) {
        Ok(IrqAllocateHandle::new(target_irq))
    } else {
        Err(Error::NotEnoughResources)
    }
}

/// The handle to a allocate irq number between [32,256), used in std and other parts in jinux
///
/// When the handle is dropped, all the callback in this will be unregistered automatically.
#[derive(Debug)]
#[must_use]
pub struct IrqAllocateHandle {
    irq_num: u8,
    irq: Arc<&'static IrqLine>,
    callbacks: Vec<IrqCallbackHandle>,
}

impl IrqAllocateHandle {
    fn new(irq_num: u8) -> Self {
        Self {
            irq_num: irq_num,
            irq: unsafe { IrqLine::acquire(irq_num) },
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

impl Drop for IrqAllocateHandle {
    fn drop(&mut self) {
        for callback in &self.callbacks {
            drop(callback)
        }
        NOT_USING_IRQ.lock().dealloc(self.irq_num as usize);
    }
}

static ID_ALLOCATOR: Mutex<RecycleAllocator> = Mutex::new(RecycleAllocator::new());

pub struct CallbackElement {
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

/// An interrupt request (IRQ) line.
#[derive(Debug)]
pub(crate) struct IrqLine {
    pub(crate) irq_num: u8,
    pub(crate) callback_list: Mutex<Vec<CallbackElement>>,
}

impl IrqLine {
    /// Acquire an interrupt request line.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as manipulating interrupt lines is
    /// considered a dangerous operation.
    pub unsafe fn acquire(irq_num: u8) -> Arc<&'static Self> {
        Arc::new(IRQ_LIST.get().unwrap().get(irq_num as usize).unwrap())
    }

    /// Get the IRQ number.
    pub fn num(&self) -> u8 {
        self.irq_num
    }

    pub fn callback_list(&self) -> MutexGuard<'_, alloc::vec::Vec<CallbackElement>> {
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
        let allocate_id = ID_ALLOCATOR.lock().alloc();
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
pub struct IrqCallbackHandle {
    irq_num: u8,
    id: usize,
    // cursor: CursorMut<'a, Box<dyn Fn(&IrqLine)+Sync+Send+'static>>
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
        a.retain(|item| if (*item).id == self.id { false } else { true });
        ID_ALLOCATOR.lock().dealloc(self.id);
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
/// ``rust
/// use jinux_frame::irq;
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

impl Drop for DisabledLocalIrqGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            irq::enable_local();
        }
    }
}
