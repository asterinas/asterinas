// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::netlink::message::{Attribute, CAttrHeader, ContinueRead},
    prelude::*,
    util::MultiRead,
};

/// Address-related attributes.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L26>.
#[derive(Debug, Clone, Copy, TryFromInt)]
#[repr(u16)]
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
enum AddrAttrClass {
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
pub enum AddrAttr {
    Address([u8; 4]),
    Local([u8; 4]),
    Label(CString),
}

impl AddrAttr {
    fn class(&self) -> AddrAttrClass {
        match self {
            AddrAttr::Address(_) => AddrAttrClass::ADDRESS,
            AddrAttr::Local(_) => AddrAttrClass::LOCAL,
            AddrAttr::Label(_) => AddrAttrClass::LABEL,
        }
    }
}

impl Attribute for AddrAttr {
    fn type_(&self) -> u16 {
        self.class() as u16
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match self {
            AddrAttr::Address(address) => address,
            AddrAttr::Local(local) => local,
            AddrAttr::Label(label) => label.as_bytes_with_nul(),
        }
    }

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<ContinueRead<Self>>
    where
        Self: Sized,
    {
        let payload_len = header.payload_len();
        reader.skip_some(payload_len);

        // GETADDR only supports dump requests. These requests do not have any attributes.
        // According to the Linux behavior, we should just ignore all the attributes.

        Ok(ContinueRead::Skipped)
    }

    fn read_all_from(
        reader: &mut dyn MultiRead,
        total_len: usize,
    ) -> Result<ContinueRead<Vec<Self>>>
    where
        Self: Sized,
    {
        reader.skip_some(total_len);

        // GETADDR only supports dump requests. These requests do not have any attributes.
        // According to the Linux behavior, we should just ignore all the attributes.

        Ok(ContinueRead::Skipped)
    }
}
