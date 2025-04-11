// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// `rtmsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L237>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
#[expect(unused)]
pub(super) struct CRtMsg {
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
