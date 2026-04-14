// SPDX-License-Identifier: MPL-2.0

//! `print` and `println` macros
//!
//! Selects one console backend for kernel prints.
//!

use alloc::{collections::btree_map::BTreeMap, fmt, string::String, sync::Arc};
use core::fmt::Write;

use aster_console::AnyConsoleDevice;
use ostd::sync::{LocalIrqDisabled, SpinLockGuard};
use spin::Once;

static CONSOLES: Once<alloc::vec::Vec<String>> = Once::new();
aster_cmdline::define_repeatable_kv_param!("console", CONSOLES);

fn selected_console_device_name() -> Option<&'static str> {
    let console_name = CONSOLES
        .get()
        .and_then(|consoles| consoles.first())
        .map(|s| s.as_str())
        .unwrap_or("tty0");

    // Translate Linux cmdline console names to internal registered console names.
    match console_name {
        "ttyS0" => Some("Uart-Console"),
        "hvc0" => Some("Virtio-Console"),
        "tty0" => Some("Framebuffer-Console"),
        _ => None,
    }
}

/// Prints the formatted arguments to the standard output.
pub fn _print(args: fmt::Arguments) {
    // We must call `all_devices_lock` instead of `all_devices` here, as `all_devices` invokes the
    // `clone` method of `String` and `Arc`, which may lead to a deadlock when there is low memory
    // in the heap. (The heap allocator will log a message when memory is low.)
    //
    // Also, holding the lock will prevent the logs from interleaving.
    let devices = aster_console::all_devices_lock();

    struct Printer<'a>(
        SpinLockGuard<'a, BTreeMap<String, Arc<dyn AnyConsoleDevice>>, LocalIrqDisabled>,
    );
    impl Write for Printer<'_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            if self.0.is_empty() {
                ostd::early_print!("{}", s);
            } else {
                // Route each message to one selected backend instead of broadcasting
                // to every registered console device.
                if let Some(device_name) = selected_console_device_name()
                    && let Some(console) = self.0.get(device_name)
                {
                    console.send(s.as_bytes());
                } else if let Some((_, console)) = self.0.first_key_value() {
                    // Fall back to the first registered console device.
                    console.send(s.as_bytes());
                }
            }
            Ok(())
        }
    }

    Printer(devices).write_fmt(args).unwrap();
}

/// Copied from Rust std: <https://github.com/rust-lang/rust/blob/master/library/std/src/macros.rs>
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::_print(format_args!($($arg)*));
    }};
}

/// Copied from Rust std: <https://github.com/rust-lang/rust/blob/master/library/std/src/macros.rs>
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::_print(format_args_nl!($($arg)*));
    }};
}
