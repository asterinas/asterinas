// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

#![allow(dead_code)]

use alloc::{boxed::Box, fmt::Debug, sync::Arc, vec::Vec};

use id_alloc::IdAlloc;
use spin::Once;
use x86_64::registers::rflags::{self, RFlags};

use super::iommu::{alloc_irt_entry, has_interrupt_remapping, IrtEntryHandle};
use crate::{
    cpu::CpuId,
    sync::{LocalIrqDisabled, Mutex, PreemptDisabled, RwLock, RwLockReadGuard, SpinLock},
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
            callback_list: RwLock::new(Vec::new()),
            bind_remapping_entry: Once::new(),
        });
    }
    IRQ_LIST.call_once(|| list);
    CALLBACK_ID_ALLOCATOR.call_once(|| Mutex::new(IdAlloc::with_capacity(256)));
    IRQ_ALLOCATOR.call_once(|| {
        // As noted in the Intel 64 and IA-32 rchitectures Software Developer’s Manual,
        // Volume 3A, Section 6.2, the first 32 interrupts are reserved for specific
        // usages. And the rest from 32 to 255 are available for external user-defined
        // interrupts.
        let mut id_alloc = IdAlloc::with_capacity(256);
        for i in 0..32 {
            id_alloc.alloc_specific(i).unwrap();
        }
        SpinLock::new(id_alloc)
    });
}

pub(crate) fn enable_local() {
    x86_64::instructions::interrupts::enable();
    // When emulated with QEMU, interrupts may not be delivered if a STI instruction is immediately
    // followed by a RET instruction. It is a BUG of QEMU, see the following patch for details.
    // https://lore.kernel.org/qemu-devel/20231210190147.129734-2-lrh2000@pku.edu.cn/
    x86_64::instructions::nop();
}

pub(crate) fn disable_local() {
    x86_64::instructions::interrupts::disable();
}

pub(crate) fn is_local_enabled() -> bool {
    (rflags::read_raw() & RFlags::INTERRUPT_FLAG.bits()) != 0
}

static CALLBACK_ID_ALLOCATOR: Once<Mutex<IdAlloc>> = Once::new();

pub struct CallbackElement {
    function: Box<dyn Fn(&TrapFrame) + Send + Sync + 'static>,
    id: usize,
}

impl CallbackElement {
    pub fn call(&self, element: &TrapFrame) {
        (self.function)(element);
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
    pub(crate) callback_list: RwLock<Vec<CallbackElement>>,
    bind_remapping_entry: Once<Arc<SpinLock<IrtEntryHandle, LocalIrqDisabled>>>,
}

impl IrqLine {
    /// Acquires an interrupt request line.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as manipulating interrupt lines is
    /// considered a dangerous operation.
    #[allow(clippy::redundant_allocation)]
    pub unsafe fn acquire(irq_num: u8) -> Arc<&'static Self> {
        let irq = Arc::new(IRQ_LIST.get().unwrap().get(irq_num as usize).unwrap());
        if has_interrupt_remapping() {
            let handle = alloc_irt_entry();
            if let Some(handle) = handle {
                irq.bind_remapping_entry.call_once(|| handle);
            }
        }
        irq
    }

    pub fn bind_remapping_entry(&self) -> Option<&Arc<SpinLock<IrtEntryHandle, LocalIrqDisabled>>> {
        self.bind_remapping_entry.get()
    }

    /// Gets the IRQ number.
    pub fn num(&self) -> u8 {
        self.irq_num
    }

    pub fn callback_list(
        &self,
    ) -> RwLockReadGuard<alloc::vec::Vec<CallbackElement>, PreemptDisabled> {
        self.callback_list.read()
    }

    /// Registers a callback that will be invoked when the IRQ is active.
    ///
    /// A handle to the callback is returned. Dropping the handle
    /// automatically unregisters the callback.
    ///
    /// For each IRQ line, multiple callbacks may be registered.
    pub fn on_active<F>(&self, callback: F) -> IrqCallbackHandle
    where
        F: Fn(&TrapFrame) + Sync + Send + 'static,
    {
        let allocated_id = CALLBACK_ID_ALLOCATOR.get().unwrap().lock().alloc().unwrap();
        self.callback_list.write().push(CallbackElement {
            function: Box::new(callback),
            id: allocated_id,
        });
        IrqCallbackHandle {
            irq_num: self.irq_num,
            id: allocated_id,
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
            .write();
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
    use crate::arch::kernel::apic::{self, Icr};

    let icr = Icr::new(
        apic::ApicId::from(cpu_id.as_usize() as u32),
        apic::DestinationShorthand::NoShorthand,
        apic::TriggerMode::Edge,
        apic::Level::Assert,
        apic::DeliveryStatus::Idle,
        apic::DestinationMode::Physical,
        apic::DeliveryMode::Fixed,
        irq_num,
    );
    apic::with_borrow(|apic| {
        apic.send_ipi(icr);
    });
}
