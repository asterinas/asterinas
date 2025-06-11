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

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<Option<Self>>
    where
        Self: Sized,
    {
        let payload_len = header.payload_len();

        // TODO: Currently, `IS_NET_BYTEORDER_MASK` and `IS_NESTED_MASK` are ignored.
        let Ok(class) = AddrAttrClass::try_from(header.type_()) else {
            // Unknown attributes should be ignored.
            // Reference: <https://docs.kernel.org/userspace-api/netlink/intro.html#unknown-attributes>.
            reader.skip_some(payload_len);
            return Ok(None);
        };

        let res = match (class, payload_len) {
            (AddrAttrClass::ADDRESS, 4) => {
                Self::Address(reader.read_val_opt::<[u8; 4]>()?.unwrap())
            }
            (AddrAttrClass::LOCAL, 4) => Self::Local(reader.read_val_opt::<[u8; 4]>()?.unwrap()),
            (AddrAttrClass::LABEL, 1..=IFNAME_SIZE) => {
                Self::Label(reader.read_cstring_with_max_len(payload_len)?)
            }

            (AddrAttrClass::ADDRESS | AddrAttrClass::LOCAL | AddrAttrClass::LABEL, _) => {
                warn!("address attribute `{:?}` contains invalid payload", class);
                return_errno_with_message!(Errno::EINVAL, "the address attribute is invalid");
            }

            (_, _) => {
                warn!("address attribute `{:?}` is not supported", class);
                reader.skip_some(payload_len);
                return Ok(None);
            }
        };

        Ok(Some(res))
    }
}
