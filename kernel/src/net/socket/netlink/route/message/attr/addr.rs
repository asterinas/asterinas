// SPDX-License-Identifier: MPL-2.0

use super::IFNAME_SIZE;
use crate::{
    net::socket::netlink::message::{Attribute, CAttrHeader},
    prelude::*,
    util::MultiRead,
};

/// Address-related attributes.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L26>.
#[derive(Debug, Clone, Copy, TryFromInt)]
#[repr(u16)]
#[expect(non_camel_case_types)]
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

    fn read_from(reader: &mut dyn MultiRead) -> Result<Self>
    where
        Self: Sized,
    {
        let header = reader.read_val::<CAttrHeader>()?;
        // TODO: Currently, `IS_NET_BYTEORDER_MASK` and `IS_NESTED_MASK` are ignored.
        let res = match AddrAttrClass::try_from(header.type_())? {
            AddrAttrClass::ADDRESS => Self::Address(reader.read_val()?),
            AddrAttrClass::LOCAL => Self::Local(reader.read_val()?),
            AddrAttrClass::LABEL => Self::Label(reader.read_cstring_with_max_len(IFNAME_SIZE)?),
            class => {
                // FIXME: Netlink should ignore all unknown attributes.
                // See the reference in `LinkAttr::read_from`.
                warn!("address attribute `{:?}` is not supported", class);
                return_errno_with_message!(Errno::EINVAL, "unsupported address attribute");
            }
        };

        Ok(res)
    }
}
