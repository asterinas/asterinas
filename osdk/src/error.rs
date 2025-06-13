// SPDX-License-Identifier: MPL-2.0

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errno {
    Cli = 1,
    CreateCrate = 2,
    GetMetadata = 3,
    AddRustToolchain = 4,
    ParseMetadata = 5,
    ExecuteCommand = 6,
    BuildCrate = 7,
    RunBundle = 8,
    BadCrateName = 9,
    NoKernelCrate = 10,
    TooManyCrates = 11,
    ExecutableNotFound = 12,
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
