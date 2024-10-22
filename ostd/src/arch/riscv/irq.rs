// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

use alloc::{boxed::Box, fmt::Debug, sync::Arc, vec::Vec};

use id_alloc::IdAlloc;
use spin::Once;

use crate::{
    cpu::CpuId,
    sync::{Mutex, PreemptDisabled, SpinLock, SpinLockGuard},
    trap::TrapFrame,
};

/// The global allocator for software defined IRQ lines.
pub(crate) static IRQ_ALLOCATOR: Once<SpinLock<IdAlloc>> = Once::new();

pub(crate) static IRQ_LIST: Once<Vec<IrqLine>> = Once::new();

pub(crate) fn init() {
    let mut list: Vec<IrqLine> = Vec::new();
    for i in 0..256 {
        list.push(IrqLine {
            irq_num: i as u8,
            callback_list: SpinLock::new(Vec::new()),
        });
    }
    IRQ_LIST.call_once(|| list);
    CALLBACK_ID_ALLOCATOR.call_once(|| Mutex::new(IdAlloc::with_capacity(256)));
    IRQ_ALLOCATOR.call_once(|| SpinLock::new(IdAlloc::with_capacity(256)));
}

pub(crate) fn enable_local() {
    unsafe { riscv::interrupt::enable() }
}

pub(crate) fn disable_local() {
    riscv::interrupt::disable();
}

pub(crate) fn is_local_enabled() -> bool {
    riscv::register::sstatus::read().sie()
}

static CALLBACK_ID_ALLOCATOR: Once<Mutex<IdAlloc>> = Once::new();

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
    pub(crate) callback_list: SpinLock<Vec<CallbackElement>>,
}

impl IrqLine {
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

    pub fn callback_list(
        &self,
    ) -> SpinLockGuard<alloc::vec::Vec<CallbackElement>, PreemptDisabled> {
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
        let allocate_id = CALLBACK_ID_ALLOCATOR.get().unwrap().lock().alloc().unwrap();
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
        CALLBACK_ID_ALLOCATOR.get().unwrap().lock().free(self.id);
    }
}

/// Sends a general inter-processor interrupt (IPI) to the specified CPU.
///
/// # Safety
///
/// The caller must ensure that the CPU ID and the interrupt number corresponds
/// to a safe function to call.
pub(crate) unsafe fn send_ipi(cpu_id: CpuId, irq_num: u8) {
    unimplemented!()
}
