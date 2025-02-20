// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// Corresponding to `rtmsg` in Linux
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
#[expect(unused)]
pub(super) struct CRouteMessage {
    family: u8,
    dst_len: u8,
    src_len: u8,
    tos: u8,
    table: u8,
    protocol: u8,
    scope: u8,
    type_: u8,
    flags: u32,
}
