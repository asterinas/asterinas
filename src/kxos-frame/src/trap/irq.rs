use crate::{prelude::*, sync::up::UPSafeCell};

use super::TrapFrame;
use lazy_static::lazy_static;
use spin::{Mutex, MutexGuard};

lazy_static! {
    /// The IRQ numbers which are not using
    /// FIXME: using alloc, dealloc instead of letting user use push and pop method.
    pub static ref NOT_USING_IRQ_NUMBER:UPSafeCell<Vec<u8>> = unsafe {UPSafeCell::new({
        let mut vector = Vec::new();
        for i in 31..256{
            vector.push(i as u8);
        }
        for i in 22..28{
            vector.push(i as u8);
        }
        vector
    })};
}

lazy_static! {
    pub static ref IRQ_LIST: Vec<IrqLine> = {
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

struct RecycleAllocator {
    current: usize,
    recycled: Vec<usize>,
}

impl RecycleAllocator {
    pub fn new() -> Self {
        RecycleAllocator {
            current: 0,
            recycled: Vec::new(),
        }
    }
    #[allow(unused)]
    pub fn alloc(&mut self) -> usize {
        if let Some(id) = self.recycled.pop() {
            id
        } else {
            self.current += 1;
            self.current - 1
        }
    }
    #[allow(unused)]
    pub fn dealloc(&mut self, id: usize) {
        assert!(id < self.current);
        assert!(
            !self.recycled.iter().any(|i| *i == id),
            "id {} has been deallocated!",
            id
        );
        self.recycled.push(id);
    }
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

/// An interrupt request (IRQ) line.
pub struct IrqLine {
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
