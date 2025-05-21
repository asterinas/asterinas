// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, process::Pid};

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct cap_user_header_t {
    pub version: u32,
    pub pid: Pid,
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct cap_user_data_t {
    pub effective: u32,
    pub permitted: u32,
    pub inheritable: u32,
}

pub const LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;
