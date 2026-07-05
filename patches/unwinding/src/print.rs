pub use crate::{eprint, eprintln, print, println};

#[doc(hidden)]
pub struct StdoutPrinter;
impl core::fmt::Write for StdoutPrinter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        unsafe { libc::printf(b"%.*s\0".as_ptr() as _, s.len() as i32, s.as_ptr()) };
        Ok(())
    }
}

#[doc(hidden)]
pub struct StderrPrinter;
impl core::fmt::Write for StderrPrinter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        unsafe { libc::write(libc::STDERR_FILENO, s.as_ptr() as _, s.len() as _) };
        Ok(())
    }
}

#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => ({
        use core::fmt::Write;
        let _ = core::writeln!($crate::print::StdoutPrinter, $($arg)*);
    })
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ({
        use core::fmt::Write;
        let _ = core::write!($crate::print::StdoutPrinter, $($arg)*);
    })
}

#[macro_export]
macro_rules! eprintln {
    ($($arg:tt)*) => ({
        use core::fmt::Write;
        let _ = core::writeln!($crate::print::StderrPrinter, $($arg)*);
    })
}

#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => ({
        use core::fmt::Write;
        let _ = core::write!($crate::print::StderrPrinter, $($arg)*);
    })
}

#[macro_export]
macro_rules! dbg {
    () => {
        $crate::eprintln!("[{}:{}]", ::core::file!(), ::core::line!())
    };
    ($val:expr $(,)?) => {
        match $val {
            tmp => {
                $crate::eprintln!("[{}:{}] {} = {:#?}",
                    ::core::file!(), ::core::line!(), ::core::stringify!($val), &tmp);
                tmp
            }
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($($crate::dbg!($val)),+,)
    };
}
