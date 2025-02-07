// SPDX-License-Identifier: MPL-2.0

//! Io Port access dispatcher.

use core::mem::size_of;

use id_alloc::IdAlloc;
use log::info;
use spin::Once;

use crate::{
    arch::device::io_port::{IoPort, PortReadWrite, IO_PORT_MAX},
    sync::SpinLock,
};

/// I/O port dispatcher that allocates I/O access to devices via PIO.
pub struct IoPortDispatcher {
    /// Each ID indicates whether a Port I/O (1B) is allocated.
    allocator: Once<SpinLock<IdAlloc>>,
}

impl IoPortDispatcher {
    /// Get the `IoPort`. Return None if any region in 'port' cannot be allocated.
    pub fn get<A>(&self, port: u16, size: u16) -> Option<IoPort<A, PortReadWrite>> {
        let mut allocator = self.allocator.get().unwrap().disable_irq().lock();
        if (port..(port + size)).any(|i| allocator.is_allocated(i as usize)) {
            return None;
        }

        for i in port..(port + size) {
            allocator.alloc_specific(i as usize);
        }
        // SAFETY: The created IoPort is guaranteed not to access system device I/O
        unsafe { Some(IoPort::new(port)) }
    }

    #[allow(dead_code)]
    /// Remove access to a specific port IO range.  
    ///
    /// All drivers in the Framework must use this method to prevent peripheral drivers from accessing illegal port IO range.
    pub(crate) fn remove<T>(&self, port: u16) {
        let mut allocator = self.allocator.get().unwrap().disable_irq().lock();
        info!(
            "Removing PIO range:{:x}..{:x}",
            port,
            port + size_of::<T>() as u16
        );
        for i in port..port + size_of::<T>() as u16 {
            allocator.alloc_specific(i as usize);
        }
    }
}

/// Io
pub static IO_PORT_DISPATCHER: IoPortDispatcher = IoPortDispatcher {
    allocator: Once::new(),
};

pub(crate) fn init() {
    IO_PORT_DISPATCHER
        .allocator
        .call_once(|| SpinLock::new(IdAlloc::with_capacity(IO_PORT_MAX as usize)));
}
