// SPDX-License-Identifier: MPL-2.0

use super::{addr::CIfaddrMsg, link::CIfinfoMsg};
use crate::prelude::*;

/// `rtgenmsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L548>.
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct CRtGenMsg {
    pub family: u8,
}

impl From<CRtGenMsg> for CIfinfoMsg {
    fn from(value: CRtGenMsg) -> Self {
        Self {
            family: value.family,
            _pad: 0,
            type_: 0,
            index: 0,
            flags: 0,
            change: 0,
        }
    }
}

impl From<CRtGenMsg> for CIfaddrMsg {
    fn from(value: CRtGenMsg) -> Self {
        Self {
            family: value.family,
            prefix_len: 0,
            flags: 0,
            scope: 0,
            index: 0,
        }
    }
}
