use crate::{prelude::*, Error};

use super::TrapFrame;
use crate::util::recycle_allocator::RecycleAllocator;
use core::fmt::Debug;
use lazy_static::lazy_static;
use spin::{Mutex, MutexGuard};

lazy_static! {
    /// The IRQ numbers which are not using
    static ref NOT_USING_IRQ: Mutex<RecycleAllocator> = Mutex::new(RecycleAllocator::with_start_max(32,256));
}

pub fn allocate_irq() -> Result<IrqAllocateHandle> {
    let irq_num = NOT_USING_IRQ.lock().alloc();
    if irq_num == usize::MAX {
        Err(Error::NotEnoughResources)
    } else {
        Ok(IrqAllocateHandle::new(irq_num as u8))
    }
}

/// The handle to a allocate irq number between [32,256), used in std and other parts in kxos
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
        F: Fn(TrapFrame) + Sync + Send + 'static,
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

lazy_static! {
    pub(crate) static ref IRQ_LIST: Vec<IrqLine> = {
        let mut list: Vec<IrqLine> = Vec::new();
        for i in 0..256 {
            list.push(IrqLine {
                irq_num: i as u8,
                callback_list: Mutex::new(Vec::new()),
            });
        }
        list
    };
}

lazy_static! {
    static ref ID_ALLOCATOR: Mutex<RecycleAllocator> = Mutex::new(RecycleAllocator::new());
}

pub struct CallbackElement {
    function: Box<dyn Fn(TrapFrame) + Send + Sync + 'static>,
    id: usize,
}

impl CallbackElement {
    pub fn call(&self, element: TrapFrame) {
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
    irq_num: u8,
    callback_list: Mutex<Vec<CallbackElement>>,
}

impl IrqLine {
    /// Acquire an interrupt request line.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as manipulating interrupt lines is
    /// considered a dangerous operation.
    pub unsafe fn acquire(irq_num: u8) -> Arc<&'static Self> {
        Arc::new(IRQ_LIST.get(irq_num as usize).unwrap())
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
        F: Fn(TrapFrame) + Sync + Send + 'static,
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
            .get(self.irq_num as usize)
            .unwrap()
            .callback_list
            .lock();
        a.retain(|item| if (*item).id == self.id { false } else { true });
        ID_ALLOCATOR.lock().dealloc(self.id);
    }
}
