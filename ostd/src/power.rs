// SPDX-License-Identifier: MPL-2.0

//! Power management.
//!
//! Each handler should be registered at most once in each category and remain panic-free.
//! Handler registration fails if a category's internal capacity is exhausted.

use core::{
    mem, ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use crate::{arch::irq::disable_local_and_halt, cpu::CpuSet};

/// An exit code that denotes the reason for restarting or powering off.
///
/// Whether or not the code is used depends on the hardware. In a virtualization environment, it
/// can be passed to the hypervisor (e.g., as QEMU's exit code). In a bare-metal environment, it
/// can be passed to the firmware. In either case, the code may be silently ignored if reporting
/// the code is not supported.
#[derive(Clone, Copy)]
pub enum ExitCode {
    /// The code that indicates a successful exit.
    Success,
    /// The code that indicates a failed exit.
    Failure,
}

/// An error returned when registering a restart or poweroff handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandlerRegistrationError {
    /// No registration slot is available.
    CapacityExhausted,
}

const MAX_NUM_HANDLERS: usize = 4;

struct HandlerRegistry {
    handlers: [AtomicPtr<()>; MAX_NUM_HANDLERS],
}

impl HandlerRegistry {
    const fn new() -> Self {
        Self {
            handlers: [const { AtomicPtr::new(ptr::null_mut()) }; MAX_NUM_HANDLERS],
        }
    }

    fn register(&self, handler: fn(ExitCode)) -> Result<(), HandlerRegistrationError> {
        let handler = handler as *mut ();

        for slot in &self.handlers {
            if slot
                .compare_exchange(
                    ptr::null_mut(),
                    handler,
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(());
            }
        }

        Err(HandlerRegistrationError::CapacityExhausted)
    }

    fn invoke(&self, code: ExitCode) -> bool {
        let mut has_handler = false;

        for slot in &self.handlers {
            let handler = slot.load(Ordering::Acquire);
            if handler.is_null() {
                continue;
            }

            has_handler = true;
            // SAFETY: Every non-null value stored in a slot is a `fn(ExitCode)` cast to a pointer,
            // and registered values are never changed or removed.
            let handler = unsafe { mem::transmute::<*mut (), fn(ExitCode)>(handler) };
            handler(code);
        }

        has_handler
    }
}

static RESTART_HANDLERS: HandlerRegistry = HandlerRegistry::new();
static FALLBACK_RESTART_HANDLERS: HandlerRegistry = HandlerRegistry::new();

/// Injects a handler that can restart the system.
///
/// Restart handlers are invoked in registration order.
pub fn inject_restart_handler(handler: fn(ExitCode)) -> Result<(), HandlerRegistrationError> {
    RESTART_HANDLERS.register(handler)
}

/// Injects a fallback handler that can restart the system.
///
/// Fallback restart handlers are invoked in registration order after all regular restart handlers
/// return.
pub fn inject_fallback_restart_handler(
    handler: fn(ExitCode),
) -> Result<(), HandlerRegistrationError> {
    FALLBACK_RESTART_HANDLERS.register(handler)
}

/// Restarts the system.
///
/// This function will not return. If no registered restart handler works, it will halt all CPUs
/// on the machine.
pub fn restart(code: ExitCode) -> ! {
    let has_restart_handler = RESTART_HANDLERS.invoke(code);
    let has_fallback_handler = FALLBACK_RESTART_HANDLERS.invoke(code);
    let has_handler = has_restart_handler || has_fallback_handler;

    if has_handler {
        crate::error!("Failed to restart the system because all restart handlers fail");
    } else {
        crate::error!("Failed to restart the system because a restart handler is missing");
    }

    machine_halt();
}

static POWEROFF_HANDLERS: HandlerRegistry = HandlerRegistry::new();

/// Injects a handler that can power off the system.
///
/// Poweroff handlers are invoked in registration order and may be called by the panic handler.
pub fn inject_poweroff_handler(handler: fn(ExitCode)) -> Result<(), HandlerRegistrationError> {
    POWEROFF_HANDLERS.register(handler)
}

/// Powers off the system.
///
/// This function will not return. If a poweroff handler is missing or not working, it will halt
/// all CPUs on the machine.
pub fn poweroff(code: ExitCode) -> ! {
    #[cfg(feature = "coverage")]
    crate::coverage::on_system_exit();

    if POWEROFF_HANDLERS.invoke(code) {
        crate::error!("Failed to power off the system because all poweroff handlers fail");
    } else {
        crate::error!("Failed to power off the system because a poweroff handler is missing");
    }

    machine_halt();
}

fn machine_halt() -> ! {
    crate::error!("Halting the machine...");

    // TODO: `inter_processor_call` may panic again (e.g., if there is an out-of-memory error). We
    // should find a way to make it panic-free.
    if let Some(ipi_sender) = crate::smp::IPI_SENDER.get() {
        ipi_sender.inter_processor_call(&CpuSet::new_full(), || disable_local_and_halt());
    }
    disable_local_and_halt();
}
