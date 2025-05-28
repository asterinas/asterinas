// SPDX-License-Identifier: MPL-2.0

//! The top half of interrupt handling.

use core::{fmt::Debug, ops::Deref};

use id_alloc::IdAlloc;
use spin::Once;

use crate::{
    arch::{
        irq::{HwIrqLine, IrqRemapping, IRQ_NUM_MAX, IRQ_NUM_MIN},
        trap::TrapFrame,
    },
    prelude::*,
    sync::{RwLock, SpinLock, WriteIrqDisabled},
    Error,
};

/// A type alias for the IRQ callback function.
pub type IrqCallbackFunction = dyn Fn(&TrapFrame) + Sync + Send + 'static;

/// An Interrupt ReQuest (IRQ) line.
///
/// Users can use [`alloc`] or [`alloc_specific`] to allocate a (specific) IRQ line.
///
/// The IRQ number is guaranteed to be an external IRQ number and users can use [`on_active`] to
/// safely register callback functions on this IRQ line. When the IRQ line is dropped, all the
/// registered callbacks will be unregistered automatically.
///
/// [`alloc`]: Self::alloc
/// [`alloc_specific`]: Self::alloc_specific
/// [`on_active`]: Self::on_active
#[derive(Debug)]
#[must_use]
pub struct IrqLine {
    inner: Arc<InnerHandle>,
    callbacks: Vec<CallbackHandle>,
}

impl IrqLine {
    /// Allocates an available IRQ line.
    pub fn alloc() -> Result<Self> {
        get_or_init_allocator()
            .lock()
            .alloc()
            .map(|id| Self::new(id as u8))
            .ok_or(Error::NotEnoughResources)
    }

    /// Allocates a specific IRQ line.
    pub fn alloc_specific(irq_num: u8) -> Result<Self> {
        get_or_init_allocator()
            .lock()
            .alloc_specific((irq_num - IRQ_NUM_MIN) as usize)
            .map(|id| Self::new(id as u8))
            .ok_or(Error::NotEnoughResources)
    }

    fn new(index: u8) -> Self {
        let inner = InnerHandle { index };
        inner.remapping.init(index + IRQ_NUM_MIN);

        Self {
            inner: Arc::new(inner),
            callbacks: Vec::new(),
        }
    }

    /// Gets the IRQ number.
    pub fn num(&self) -> u8 {
        self.inner.index + IRQ_NUM_MIN
    }

    /// Registers a callback that will be invoked when the IRQ is active.
    ///
    /// For each IRQ line, multiple callbacks may be registered.
    pub fn on_active<F>(&mut self, callback: F)
    where
        F: Fn(&TrapFrame) + Sync + Send + 'static,
    {
        let callback_handle = {
            let callback_box = Box::new(callback);
            let callback_addr = core::ptr::from_ref(&*callback_box).addr();

            let mut callbacks = self.inner.callbacks.write();
            callbacks.push(callback_box);

            CallbackHandle {
                irq_index: self.inner.index,
                callback_addr,
            }
        };

        self.callbacks.push(callback_handle);
    }

    /// Checks if there are no registered callbacks.
    pub fn is_empty(&self) -> bool {
        self.callbacks.is_empty()
    }

    /// Gets the remapping index of the IRQ line.
    ///
    /// This method will return `None` if interrupt remapping is disabled or
    /// not supported by the architecture.
    pub fn remapping_index(&self) -> Option<u16> {
        self.inner.remapping.remapping_index()
    }
}

impl Clone for IrqLine {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            callbacks: Vec::new(),
        }
    }
}

struct Inner {
    callbacks: RwLock<Vec<Box<IrqCallbackFunction>>, WriteIrqDisabled>,
    remapping: IrqRemapping,
}

impl Inner {
    const fn new() -> Self {
        Self {
            callbacks: RwLock::new(Vec::new()),
            remapping: IrqRemapping::new(),
        }
    }
}

const NUMBER_OF_IRQS: usize = (IRQ_NUM_MAX - IRQ_NUM_MIN) as usize + 1;

static INNERS: [Inner; NUMBER_OF_IRQS] = [const { Inner::new() }; NUMBER_OF_IRQS];
static ALLOCATOR: Once<SpinLock<IdAlloc>> = Once::new();

fn get_or_init_allocator() -> &'static SpinLock<IdAlloc> {
    ALLOCATOR.call_once(|| SpinLock::new(IdAlloc::with_capacity(NUMBER_OF_IRQS)))
}

/// A handle for an allocated IRQ line.
///
/// When the handle is dropped, the IRQ line will be released automatically.
#[must_use]
#[derive(Debug)]
struct InnerHandle {
    index: u8,
}

impl Deref for InnerHandle {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &INNERS[self.index as usize]
    }
}

impl Drop for InnerHandle {
    fn drop(&mut self) {
        ALLOCATOR.get().unwrap().lock().free(self.index as usize);
    }
}

/// A handle for a registered callback on an IRQ line.
///
/// When the handle is dropped, the callback will be unregistered automatically.
#[must_use]
#[derive(Debug)]
struct CallbackHandle {
    irq_index: u8,
    callback_addr: usize,
}

impl Drop for CallbackHandle {
    fn drop(&mut self) {
        let mut callbacks = INNERS[self.irq_index as usize].callbacks.write();

        let pos = callbacks
            .iter()
            .position(|element| core::ptr::from_ref(&**element).addr() == self.callback_addr);
        let _ = callbacks.swap_remove(pos.unwrap());
    }
}

pub(super) fn process(trap_frame: &TrapFrame, hw_irq_line: &HwIrqLine) {
    let inner = &INNERS[(hw_irq_line.irq_num() - IRQ_NUM_MIN) as usize];
    for callback in &*inner.callbacks.read() {
        callback(trap_frame);
    }
    hw_irq_line.ack();
}

#[cfg(ktest)]
mod test {
    use super::*;

    const IRQ_NUM: u8 = 64;
    const IRQ_INDEX: usize = (IRQ_NUM - IRQ_NUM_MIN) as usize;

    #[ktest]
    fn alloc_and_free_irq() {
        let irq_line = IrqLine::alloc_specific(IRQ_NUM).unwrap();
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_err());

        let irq_line_cloned = irq_line.clone();
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_err());

        drop(irq_line);
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_err());

        drop(irq_line_cloned);
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_ok());
    }

    #[ktest]
    fn register_and_unregister_callback() {
        let mut irq_line = IrqLine::alloc_specific(IRQ_NUM).unwrap();
        let mut irq_line_cloned = irq_line.clone();

        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 0);

        irq_line.on_active(|_| {});
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 1);

        irq_line_cloned.on_active(|_| {});
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 2);

        irq_line_cloned.on_active(|_| {});
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 3);

        drop(irq_line);
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 2);

        drop(irq_line_cloned);
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 0);
    }
}
