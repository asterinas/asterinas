pub mod termio;

use crate::define_ioctl_cmd;
use crate::prelude::*;

define_ioctl_cmd! {
    // Get terminal attributes
    TCGETS = 0x5401,
    TCSETS = 0x5402,
    // Get the process group ID of the foreground process group on this terminal
    TIOCGPGRP = 0x540f,
    // Set the foreground process group ID of this terminal.
    TIOCSPGRP = 0x5410,
    // Set window size
    TIOCGWINSZ = 0x5413,
    TIOCSWINSZ = 0x5414
}

#[macro_export]
macro_rules! define_ioctl_cmd {
    ($($name: ident = $value: expr),*) => {
        #[repr(u32)]
        #[derive(Debug, Clone, Copy)]
        pub enum IoctlCmd {
            $($name = $value,)*
        }

        $(
            pub const $name: u32 = $value;
        )*

        impl TryFrom<u32> for IoctlCmd {
            type Error = Error;
            fn try_from(value: u32) -> Result<Self> {
                match value {
                    $($name => Ok(IoctlCmd::$name),)*
                    _ => return_errno!(Errno::EINVAL),
                }
            }
        }
    }
}
