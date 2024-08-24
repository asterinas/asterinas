// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_rights::{ReadOp, TRightSet, WriteOp};
use aster_util::safe_ptr::SafePtr;
use ostd::{
    io_mem::IoMem,
    sync::RwLock,
    trap::{IrqCallbackFunction, IrqLine, TrapFrame},
};

/// Multiplexing Irqs. The two interrupt types (configuration space change and queue interrupt)
/// of the virtio-mmio device share the same IRQ, so `MultiplexIrq` are used to distinguish them.
/// Besides, virtio-mmio requires ack_interrupt after interrupt is handled.
pub struct MultiplexIrq {
    irq: IrqLine,
    queue_callbacks: Vec<Box<IrqCallbackFunction>>,
    cfg_callbacks: Vec<Box<IrqCallbackFunction>>,
    interrupt_ack: SafePtr<u32, IoMem, TRightSet<WriteOp>>,
    interrupt_status: SafePtr<u32, IoMem, TRightSet<ReadOp>>,
}

impl MultiplexIrq {
    pub fn new(
        irq: IrqLine,
        interrupt_ack: SafePtr<u32, IoMem, TRightSet<WriteOp>>,
        interrupt_status: SafePtr<u32, IoMem, TRightSet<ReadOp>>,
    ) -> Arc<RwLock<Self>> {
        let irq = Arc::new(RwLock::new(Self {
            irq,
            queue_callbacks: Vec::new(),
            cfg_callbacks: Vec::new(),
            interrupt_ack,
            interrupt_status,
        }));
        // Holding a weak reference to prevent memory leakage due to
        // circular reference.
        let weak = Arc::downgrade(&irq);
        let mut lock = irq.write();
        let callback = move |trap_frame: &TrapFrame| {
            let Some(multiplex_irq) = weak.upgrade() else {
                return;
            };
            let irq = multiplex_irq.read();
            let interrupt_status = irq.interrupt_status.read_once().unwrap();
            let callbacks = if interrupt_status & 0x01 == 1 {
                // Used buffer notification
                &irq.queue_callbacks
            } else {
                // Configuration Change Notification
                &irq.cfg_callbacks
            };
            for callback in callbacks.iter() {
                callback.call((trap_frame,));
            }
            irq.interrupt_ack.write_once(&interrupt_status).unwrap();
        };
        lock.irq.on_active(callback);
        drop(lock);
        irq
    }

    pub fn register_queue_callback(&mut self, func: Box<IrqCallbackFunction>) {
        self.queue_callbacks.push(func);
    }

    pub fn register_cfg_callback(&mut self, func: Box<IrqCallbackFunction>) {
        self.cfg_callbacks.push(func);
    }
}

impl Debug for MultiplexIrq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MultiplexIrq")
            .field("irq", &self.irq)
            .field("interrupt_ack", &self.interrupt_ack)
            .field("interrupt_status", &self.interrupt_status)
            .finish()
    }
}
