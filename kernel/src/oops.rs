// SPDX-License-Identifier: MPL-2.0

//! Kernel "oops" handling.
//!
//! In Asterinas, a Rust panic leads to a kernel "oops". A kernel oops behaves
//! as an exceptional control flow event. If kernel oopses happened too many
//! times, the kernel panics and the system gets halted.
//!
//! Though we can recover from the Rust panics. It is generally not recommended
//! to make Rust panics as a general exception handling mechanism. Handling
//! exceptions with [`Result`] is more idiomatic.

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
};
use core::{
    result::Result,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ostd::panic;

use crate::{current_thread, Thread};

// TODO: Control the kernel commandline parsing from the kernel crate.
// In Linux it can be dynamically changed by writing to
// `/proc/sys/kernel/panic`.
static PANIC_ON_OOPS: AtomicBool = AtomicBool::new(true);

/// The kernel "oops" information.
pub struct OopsInfo {
    /// The "oops" message.
    pub message: String,
    /// The thread where the "oops" happened.
    pub thread: Arc<Thread>,
}

/// Executes the given function and catches any panics that occur.
///
/// All the panics in the given function will be regarded as oops. If a oops
/// happens, this function returns `None`. Otherwise, it returns the return
/// value of the given function.
///
/// If the kernel is configured to panic on oops, this function will not return
/// when a oops happens.
pub fn catch_panics_as_oops<F, R>(f: F) -> Result<R, OopsInfo>
where
    F: FnOnce() -> R,
{
    if PANIC_ON_OOPS.load(Ordering::Relaxed) {
        return Ok(f());
    }

    let result = panic::catch_unwind(f);

    match result {
        Ok(result) => Ok(result),
        Err(err) => {
            let info = err.downcast::<OopsInfo>().unwrap();

            log::error!("Oops! {}", info.message);

            let count = OOPS_COUNT.fetch_add(1, Ordering::Relaxed);
            if count >= MAX_OOPS_COUNT {
                // Too many oops. Panic the kernel.
                //
                // Note that for nested `catch_panics_as_oops` it still works as
                // expected. The outer `catch_panics_as_oops` will catch the panic
                // and found that the oops count is too high, then panic the kernel.
                panic!("Too many oops. The kernel panics.");
            }

            Err(*info)
        }
    }
}

/// The maximum number of oops allowed before the kernel panics.
///
/// It is the same as Linux's default value.
const MAX_OOPS_COUNT: usize = 10_000;

static OOPS_COUNT: AtomicUsize = AtomicUsize::new(0);

#[ostd::panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let message = info.message().to_string();
    let thread = current_thread!();

    // Raise the panic and expect it to be caught.
    // TODO: eliminate the need for heap allocation.
    panic::begin_panic(Box::new(OopsInfo {
        message: message.clone(),
        thread,
    }));

    // Halt the system if the panic is not caught.
    log::error!("Uncaught panic! {:#?}", message);

    panic::print_stack_trace();
    panic::abort();
}
