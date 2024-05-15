// SPDX-License-Identifier: MPL-2.0

//! Form file paths within and across FSes with dentries and mount points.

pub use dentry::{Dentry, DentryKey};
pub use mount::MountNode;

mod dentry;
mod mount;
