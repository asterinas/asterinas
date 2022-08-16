use crate::prelude::*;

use alloc::collections::{linked_list::CursorMut, LinkedList};
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::{
    set_general_handler,
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame, InterruptStackFrameValue},
};
lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        set_general_handler!(&mut idt, my_general_hander);
        idt
    };
}
lazy_static! {
    static ref IRQ_LIST: Vec<IrqLine> = {
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

pub fn init() {
    IDT.load();
}

fn my_general_hander(stack_frame: InterruptStackFrame, index: u8, error_code: Option<u64>) {
    let irq_line = IRQ_LIST.get(index as usize).unwrap();
    let callback_functions = irq_line.callback_list.lock();
    for callback_function in callback_functions.iter() {
        callback_function.function.call((InterruptInformation {
            interrupt_stack_frame: *stack_frame,
            error_code,
        },));
    }
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

struct CallbackElement {
    function: Box<dyn Fn(InterruptInformation) + Send + Sync + 'static>,
    id: usize,
}

/// An interrupt request (IRQ) line.
pub struct IrqLine {
    irq_num: u8,
    callback_list: Mutex<Vec<CallbackElement>>,
}

#[derive(Debug)]
pub struct InterruptInformation {
    pub interrupt_stack_frame: InterruptStackFrameValue,
    pub error_code: Option<u64>,
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

    /// Register a callback that will be invoked when the IRQ is active.
    ///
    /// A handle to the callback is returned. Dropping the handle
    /// automatically unregisters the callback.
    ///
    /// For each IRQ line, multiple callbacks may be registered.
    pub fn on_active<F>(&self, callback: F) -> IrqCallbackHandle
    where
        F: Fn(InterruptInformation) + Sync + Send + 'static,
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

pub fn test1(irq: &IrqLine) {
    let a: Arc<&'static IrqLine>;
    unsafe {
        a = IrqLine::acquire(1);
    }
    a.on_active(test_callback);
}

pub fn test_callback(ira: InterruptInformation) {}

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
