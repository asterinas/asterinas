// SPDX-License-Identifier: MPL-2.0

//! Console output.

use core::fmt::Arguments;

/// Prints formatted arguments to the console.
///
/// The message printed by this function will exclusively be printed without
/// being mixed with other messages. However two distinct executions of this
/// function on the same core may not be contiguous.
///
/// IRQs are disabled while printing. So do not print long messages.
pub fn early_print(args: Arguments) {
    use crate::sync::SpinLock;
    static MESSAGE_LOCK: SpinLock<()> = SpinLock::new(());
    let _lock = MESSAGE_LOCK.lock_irq_disabled();
    crate::arch::serial::print(args);
}

/// Prints to the console atomically.
///
/// This method suffers from disabling IRQs while printing, so it should not
/// be used for long messages.
#[macro_export]
macro_rules! early_print {
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::early_print(format_args!($fmt $(, $($arg)+)?))
    }
}

/// Prints to the console atomically, with a newline.
///
/// This method suffers from disabling IRQs while printing, so it should not
/// be used for long messages.
#[macro_export]
macro_rules! early_println {
    () => { $crate::early_print!("\n") };
    ($fmt: literal $(, $($arg: tt)+)?) => {
        $crate::console::early_print(format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
    }
}
