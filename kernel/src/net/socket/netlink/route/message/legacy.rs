// SPDX-License-Identifier: MPL-2.0

use super::{CAddrMessage, CLinkMessage};
use crate::prelude::*;

/// The lagacy message format
#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct CRtGenMessage {
    pub family: u8,
}

impl From<CRtGenMessage> for CLinkMessage {
    fn from(value: CRtGenMessage) -> Self {
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

impl From<CRtGenMessage> for CAddrMessage {
    fn from(value: CRtGenMessage) -> Self {
        Self {
            family: value.family,
            prefix_len: 0,
            flags: 0,
            scope: 0,
            index: 0,
        }
    }
}
