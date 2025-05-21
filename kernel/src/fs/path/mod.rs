// SPDX-License-Identifier: MPL-2.0

//! Form file paths within and across FSes with dentries and mount points.

pub use dentry::{Dentry, DentryKey};
pub use mount::MountNode;

mod dentry;
mod mount;

/// Checks if the file name is ".", indicating it's the current directory.
pub const fn is_dot(filename: &str) -> bool {
    let name_bytes = filename.as_bytes();
    name_bytes.len() == 1 && name_bytes[0] == DOT_BYTE
}

/// Checks if the file name is "..", indicating it's the parent directory.
pub const fn is_dotdot(filename: &str) -> bool {
    let name_bytes = filename.as_bytes();
    name_bytes.len() == 2 && name_bytes[0] == DOT_BYTE && name_bytes[1] == DOT_BYTE
}

/// Checks if the file name is "." or "..", indicating it's the current or parent directory.
pub const fn is_dot_or_dotdot(filename: &str) -> bool {
    let name_bytes = filename.as_bytes();
    let name_len = name_bytes.len();
    if name_len == 1 {
        name_bytes[0] == DOT_BYTE
    } else if name_len == 2 {
        name_bytes[0] == DOT_BYTE && name_bytes[1] == DOT_BYTE
    } else {
        false
    }
}

const DOT_BYTE: u8 = b'.';
