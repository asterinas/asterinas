use core::fmt::Arguments;

/// Print log message
/// This function should *NOT* be directly called.
/// Instead, print logs with macros.
#[cfg(not(feature = "serial_print"))]
#[doc(hidden)]
pub fn log_print(args: Arguments) {
    use crate::device::framebuffer::WRITER;
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    interrupts::without_interrupts(|| {
        WRITER.lock().as_mut().unwrap().write_fmt(args).unwrap();
    });
}

/// Print log message
/// This function should *NOT* be directly called.
/// Instead, print logs with macros.
#[cfg(feature = "serial_print")]
#[doc(hidden)]
pub fn log_print(args: Arguments) {
    use crate::device::serial::SERIAL;
    use core::fmt::Write;
    use x86_64::instructions::interrupts;
    interrupts::without_interrupts(|| {
        SERIAL
            .lock()
            .write_fmt(args)
            .expect("Printing to serial failed");
    });
}

/// This macro should not be directly called.
#[macro_export]
macro_rules! log_print {
    ($($arg:tt)*) => {
        $crate::log::log_print(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => {
        $crate::log_print!("[trace]:");
        $crate::log_print!($($arg)*);
        $crate::log_print!("\n");
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::log_print!("[debug]:");
        $crate::log_print!($($arg)*);
        $crate::log_print!("\n");
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        ($crate::log_print!("[info]:"));
        ($crate::log_print!($($arg)*));
        ($crate::log_print!("\n"));
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log_print!("[warn]:");
        $crate::log_print!($($arg)*);
        $crate::log_print!("\n");
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::log_print!("[error]:");
        $crate::log_print!($($arg)*);
        $crate::log_print!("\n");
    };
}
