// SPDX-License-Identifier: MPL-2.0

use super::*;

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

// FIXME: This attributes are for ipv4 only. View `ifa_ipv4_policy` in Linux
define_attribute!(AddrAttrClass::ADDRESS, IfaAddress, [u8; 4]);
define_attribute!(AddrAttrClass::LOCAL, IfaLocal, [u8; 4]);
define_attribute!(AddrAttrClass::LABEL, IfaLabel, CString, IFNAME_SIZE);

pub fn read_addr_attrs(
    mut attrs_len: usize,
    reader: &mut dyn MultiRead,
) -> Result<Vec<Box<dyn NlAttr>>> {
    read_attrs_util!(attrs_len, reader, AddrAttrClass,
        (
            AddrAttrClass::ADDRESS => IfaAddress,
            AddrAttrClass::LOCAL => IfaLocal,
            AddrAttrClass::LABEL => IfaLabel
        )
    )
}
