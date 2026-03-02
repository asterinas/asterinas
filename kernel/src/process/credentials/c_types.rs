// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, process::Pid};

/// `struct __user_cap_header_struct` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18.6/source/include/uapi/linux/capability.h#L40>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CUserCapHeader {
    pub version: u32,
    pub pid: Pid,
}

/// `struct __user_cap_data_struct` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18.6/source/include/uapi/linux/capability.h#L40>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CUserCapData {
    pub effective: u32,
    pub permitted: u32,
    pub inheritable: u32,
}

pub const LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;
