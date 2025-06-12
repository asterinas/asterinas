// SPDX-License-Identifier: MPL-2.0

use super::IFNAME_SIZE;
use crate::{
    net::socket::netlink::message::{Attribute, CAttrHeader},
    prelude::*,
    util::MultiRead,
};

/// Link-level attributes.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_link.h#L297>.
#[derive(Debug, Clone, Copy, TryFromInt)]
#[repr(u16)]
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
enum LinkAttrClass {
    UNSPEC = 0,
    ADDRESS = 1,
    BROADCAST = 2,
    IFNAME = 3,
    MTU = 4,
    LINK = 5,
    QDISC = 6,
    STATS = 7,
    COST = 8,
    PRIORITY = 9,
    MASTER = 10,
    /// Wireless Extension event
    WIRELESS = 11,
    /// Protocol specific information for a link
    PROTINFO = 12,
    TXQLEN = 13,
    MAP = 14,
    WEIGHT = 15,
    OPERSTATE = 16,
    LINKMODE = 17,
    LINKINFO = 18,
    NET_NS_PID = 19,
    IFALIAS = 20,
    /// Number of VFs if device is SR-IOV PF
    NUM_VF = 21,
    VFINFO_LIST = 22,
    STATS64 = 23,
    VF_PORTS = 24,
    PORT_SELF = 25,
    AF_SPEC = 26,
    /// Group the device belongs to
    GROUP = 27,
    NET_NS_FD = 28,
    /// Extended info mask, VFs, etc.
    EXT_MASK = 29,
    /// Promiscuity count: > 0 means acts PROMISC
    PROMISCUITY = 30,
    NUM_TX_QUEUES = 31,
    NUM_RX_QUEUES = 32,
    CARRIER = 33,
    PHYS_PORT_ID = 34,
    CARRIER_CHANGES = 35,
    PHYS_SWITCH_ID = 36,
    LINK_NETNSID = 37,
    PHYS_PORT_NAME = 38,
    PROTO_DOWN = 39,
    GSO_MAX_SEGS = 40,
    GSO_MAX_SIZE = 41,
    PAD = 42,
    XDP = 43,
    EVENT = 44,
    NEW_NETNSID = 45,
    IF_NETNSID = 46,
    CARRIER_UP_COUNT = 47,
    CARRIER_DOWN_COUNT = 48,
    NEW_IFINDEX = 49,
    MIN_MTU = 50,
    MAX_MTU = 51,
    PROP_LIST = 52,
    /// Alternative ifname
    ALT_IFNAME = 53,
    PERM_ADDRESS = 54,
    PROTO_DOWN_REASON = 55,
    PARENT_DEV_NAME = 56,
    PARENT_DEV_BUS_NAME = 57,
}

#[derive(Debug)]
pub enum LinkAttr {
    Name(CString),
    Mtu(u32),
    TxqLen(u32),
    LinkMode(u8),
    ExtMask(RtExtFilter),
}

impl LinkAttr {
    fn class(&self) -> LinkAttrClass {
        match self {
            LinkAttr::Name(_) => LinkAttrClass::IFNAME,
            LinkAttr::Mtu(_) => LinkAttrClass::MTU,
            LinkAttr::TxqLen(_) => LinkAttrClass::TXQLEN,
            LinkAttr::LinkMode(_) => LinkAttrClass::LINKMODE,
            LinkAttr::ExtMask(_) => LinkAttrClass::EXT_MASK,
        }
    }
}

impl Attribute for LinkAttr {
    fn type_(&self) -> u16 {
        self.class() as u16
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match self {
            LinkAttr::Name(name) => name.as_bytes_with_nul(),
            LinkAttr::Mtu(mtu) => mtu.as_bytes(),
            LinkAttr::TxqLen(txq_len) => txq_len.as_bytes(),
            LinkAttr::LinkMode(link_mode) => link_mode.as_bytes(),
            LinkAttr::ExtMask(ext_filter) => ext_filter.as_bytes(),
        }
    }

    fn read_from(reader: &mut dyn MultiRead) -> Result<Self>
    where
        Self: Sized,
    {
        let header = reader.read_val_opt::<CAttrHeader>()?.unwrap();

        // TODO: Currently, `IS_NET_BYTEORDER_MASK` and `IS_NESTED_MASK` are ignored.
        let res = match LinkAttrClass::try_from(header.type_())? {
            LinkAttrClass::IFNAME => Self::Name(reader.read_cstring_with_max_len(IFNAME_SIZE)?),
            LinkAttrClass::MTU => Self::Mtu(reader.read_val_opt()?.unwrap()),
            LinkAttrClass::TXQLEN => Self::TxqLen(reader.read_val_opt()?.unwrap()),
            LinkAttrClass::LINKMODE => Self::LinkMode(reader.read_val_opt()?.unwrap()),
            LinkAttrClass::EXT_MASK => Self::ExtMask(reader.read_val_opt()?.unwrap()),
            class => {
                // FIXME: Netlink should ignore all unknown attributes.
                // But how to decide the payload type if the class is unknown?
                // Reference: https://docs.kernel.org/userspace-api/netlink/intro.html#unknown-attributes
                warn!("link attribute `{:?}` is not supported", class);
                return_errno_with_message!(Errno::EINVAL, "unsupported link attribute");
            }
        };

        Ok(res)
    }
}

bitflags! {
    /// New extended info filters for [`NlLinkAttr::ExtMask`].
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L819>.
    #[repr(C)]
    #[derive(Pod)]
    pub struct RtExtFilter: u32 {
        const VF = 1 << 0;
        const BRVLAN = 1 << 1;
        const BRVLAN_COMPRESSED = 1 << 2;
        const SKIP_STATS = 1 << 3;
        const MRP = 1 << 4;
        const CFM_CONFIG = 1 << 5;
        const CFM_STATUS = 1 << 6;
        const MST = 1 << 7;
    }
}
