// SPDX-License-Identifier: MPL-2.0

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errno {
    CreateCrate = 1,
    GetMetadata = 2,
    AddRustToolchain = 3,
    ParseMetadata = 4,
    ExecuteCommand = 5,
    BuildCrate = 6,
    RunBundle = 7,
}

/// Print error message to console
#[macro_export]
macro_rules! error_msg {
    () => {
        std::eprint!("")
    };
    ($($arg:tt)*) => {{
        std::eprint!("[Error]: ");
        std::eprint!($($arg)*);
        std::eprint!("\n")
    }};
}

/// Print warning message to console
#[macro_export]
macro_rules! warn_msg {
    () => {
        std::eprint!("")
    };
    ($($arg:tt)*) => {{
        std::eprint!("[Warn]: ");
        std::eprint!($($arg)*);
        std::eprint!("\n")
    }};
}
