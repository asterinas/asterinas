// SPDX-License-Identifier: MPL-2.0

use super::{CNlAttrHeader, NlAttr, ATTRIBUTE_TYPE_MASK, IFNAME_SIZE};
use crate::prelude::*;

/// Address-related attributes
/// Ref: https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L26
#[derive(Debug, Clone, Copy, TryFromInt)]
#[repr(u16)]
#[expect(non_camel_case_types)]
pub enum AddrAttrClass {
    UNSPEC = 0,
    ADDRESS = 1,
    LOCAL = 2,
    LABEL = 3,
    BROADCAST = 4,
    ANYCAST = 5,
    CACHEINFO = 6,
    MULTICAST = 7,
    FLAGS = 8,
    RT_PRIORITY = 9,
    TARGET_NETNSID = 10,
}

#[derive(Debug)]
pub enum NlAddrAttr {
    Address([u8; 4]),
    Local([u8; 4]),
    Label(CString),
}

impl NlAddrAttr {
    fn class(&self) -> AddrAttrClass {
        match self {
            NlAddrAttr::Address(_) => AddrAttrClass::ADDRESS,
            NlAddrAttr::Local(_) => AddrAttrClass::LOCAL,
            NlAddrAttr::Label(_) => AddrAttrClass::LABEL,
        }
    }
}

impl NlAttr for NlAddrAttr {
    fn type_(&self) -> u16 {
        self.class() as u16 & ATTRIBUTE_TYPE_MASK
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match self {
            NlAddrAttr::Address(address) => address.as_bytes(),
            NlAddrAttr::Local(local) => local.as_bytes(),
            NlAddrAttr::Label(label) => label.as_bytes_with_nul(),
        }
    }

    fn read_from(reader: &mut VmReader) -> Result<Self>
    where
        Self: Sized,
    {
        let header = reader.read_val::<CNlAttrHeader>()?;
        let res = match AddrAttrClass::try_from(header.type_())? {
            AddrAttrClass::ADDRESS => Self::Address(reader.read_val()?),
            AddrAttrClass::LOCAL => Self::Local(reader.read_val()?),
            AddrAttrClass::LABEL => Self::Label(reader.read_cstring_with_max_len(IFNAME_SIZE)?),
            _ => todo!(),
        };

        Ok(res)
    }
}
