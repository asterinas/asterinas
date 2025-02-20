// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    util::{MultiRead, MultiWrite},
};

#[derive(Debug, Clone, Copy, Pod, Getters)]
#[repr(C)]
#[getset(get = "pub")]
pub struct CNetlinkAttrHeader {
    len: u16,
    type_: u16,
}

const IS_NESTED_MASK: u16 = 1u16 << 15;
const IS_NET_BYTEORDER_MASK: u16 = 1u16 << 14;
const ATTRIBUTE_TYPE_MASK: u16 = !(IS_NESTED_MASK | IS_NET_BYTEORDER_MASK);

/// Link-level addributes
/// Ref: https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_link.h#L297
#[derive(Debug, Clone, Copy, TryFromInt)]
#[repr(u16)]
#[allow(non_camel_case_types)]
pub enum LinkAttrType {
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
    /// Extended info mask, VFs, etc
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

#[derive(Debug, Clone, Copy, TryFromInt)]
#[repr(u16)]
#[expect(non_camel_case_types)]
pub enum AddrAttrType {
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

pub trait AttrOps: Debug + Send + Sync {
    /// The attribute payload len(w/o padding)
    fn payload_len(&self) -> usize;

    /// The padding len of attribute payload.
    ///
    /// The padding is added to ensure the total attribute length is the multiple of 4.
    fn payload_padding_len(&self) -> usize {
        (4 - (self.payload_len() & 0x3)) & 0x3
    }

    /// The total len of the attribute(header + payload, w/o padding)
    fn total_len(&self) -> usize {
        core::mem::size_of::<CNetlinkAttrHeader>() + self.payload_len()
    }

    /// The total len of the attribute(header + payload + payload padding)
    fn total_len_with_padding(&self) -> usize {
        self.total_len() + self.payload_padding_len()
    }

    /// Writes the attribute to user space.
    ///
    /// If this operation returns success, the function will returns the actual write len.
    fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<usize>;

    fn as_any(&self) -> &dyn Any;
}

pub trait ReadAttrFromUser: Sized + AttrOps {
    type Payload;

    fn new(payload: Self::Payload) -> Self;
    fn read_payload_from_user(reader: &mut dyn MultiRead, len: usize) -> Result<Self::Payload>;
    fn read_from_user(reader: &mut dyn MultiRead, header: &CNetlinkAttrHeader) -> Result<Self> {
        let payload = {
            let len = header.len as usize - core::mem::size_of_val(header);
            Self::read_payload_from_user(reader, len)?
        };

        let res = Self::new(payload);

        let padding_len = res.payload_padding_len();
        if padding_len > 0 {
            let mut padding = vec![0u8; padding_len];
            let _ = reader.read(&mut VmWriter::from(padding.as_mut_slice()));
        }

        Ok(res)
    }
}

macro_rules! define_attribute_with_primitive {
    ($attr_var: expr, $attr_name:ident, $attr_type:ty) => {
        #[derive(Debug)]
        pub struct $attr_name {
            pub value: $attr_type,
        }

        impl AttrOps for $attr_name {
            fn payload_len(&self) -> usize {
                core::mem::size_of::<$attr_type>()
            }

            fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
                let type_ = $attr_var as u16;
                let header = CNetlinkAttrHeader {
                    type_,
                    len: self.total_len() as u16,
                };

                println!("attr header = {:?}", header);

                writer.write_val(&header)?;
                writer.write_val(&self.value)?;

                // We need to insert padding to ensure th attributes ends with addr of multiple of 4.
                let padding_len = self.payload_padding_len();
                if padding_len != 0 {
                    let padding = vec![0u8; padding_len];
                    writer.write(&mut VmReader::from(padding.as_slice()))?;
                }

                Ok(self.total_len() + padding_len)
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl ReadAttrFromUser for $attr_name {
            type Payload = $attr_type;

            fn new(value: $attr_type) -> Self {
                Self { value }
            }

            fn read_payload_from_user(
                reader: &mut dyn MultiRead,
                len: usize,
            ) -> Result<Self::Payload> {
                if len != core::mem::size_of::<$attr_type>() {
                    return_errno_with_message!(Errno::EINVAL, "invalid length");
                }

                reader.read_val::<$attr_type>()
            }
        }
    };
}

macro_rules! define_attribute_with_cstring {
    ($attr_var: expr, $attr_name:ident, $max_len: expr) => {
        #[derive(Debug)]
        pub struct $attr_name {
            pub value: CString,
        }

        impl AttrOps for $attr_name {
            fn payload_len(&self) -> usize {
                self.value.as_bytes_with_nul().len()
            }

            fn write_to_user(&self, writer: &mut dyn MultiWrite) -> Result<usize> {
                let type_ = $attr_var as u16;
                let header = CNetlinkAttrHeader {
                    type_,
                    len: self.total_len() as u16,
                };

                writer.write_val(&header)?;
                writer.write(&mut VmReader::from(self.value.as_bytes_with_nul()))?;

                let padding_len = self.payload_padding_len();
                if padding_len != 0 {
                    let padding = vec![0u8; padding_len];
                    writer.write(&mut VmReader::from(padding.as_slice()))?;
                }

                Ok(self.total_len() + padding_len)
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl ReadAttrFromUser for $attr_name {
            type Payload = CString;

            fn new(value: Self::Payload) -> Self {
                Self { value }
            }

            fn read_payload_from_user(
                reader: &mut dyn MultiRead,
                len: usize,
            ) -> Result<Self::Payload> {
                let max_len = $max_len.min(len);
                reader.read_cstring_with_max_len(max_len)
            }
        }
    };
}

define_attribute_with_primitive!(LinkAttrType::MTU, Mtu, u32);
define_attribute_with_primitive!(LinkAttrType::TXQLEN, TxqLen, u32);
define_attribute_with_primitive!(LinkAttrType::LINKMODE, LinkMode, u8);

define_attribute_with_cstring!(LinkAttrType::IFNAME, IfName, IFNAME_SIZE);

// FIXME: This attributes are for ipv4 only. View `ifa_ipv4_policy` in Linux
define_attribute_with_primitive!(AddrAttrType::LOCAL, IfaLocal, u32);
define_attribute_with_primitive!(AddrAttrType::ADDRESS, IfaAddress, u32);
define_attribute_with_cstring!(AddrAttrType::LABEL, IfaLable, IFNAME_SIZE);

const IFNAME_SIZE: usize = 16;
