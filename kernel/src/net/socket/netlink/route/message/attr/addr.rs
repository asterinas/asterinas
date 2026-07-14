// SPDX-License-Identifier: MPL-2.0

use core::net::{IpAddr, Ipv4Addr};

use zerocopy::Immutable;

use crate::{
    net::socket::netlink::{
        message::{Attribute, CAttrHeader, ContinueRead},
        route::message::AddrMessageFlags,
    },
    prelude::*,
    util::MultiRead,
};

/// Address-related attributes.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/if_addr.h#L26>.
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
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
    PROTO = 11,
}

/// The source protocol of an interface address.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/if_addr.h#L73>.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Immutable, IntoBytes)]
pub enum AddrProtocol {
    /// An unspecified address source.
    #[expect(dead_code)]
    Unspecified = 0,
    /// A loopback address configured by the kernel.
    KernelLoopback = 1,
    /// An address configured by the kernel from a Router Advertisement.
    #[expect(dead_code)]
    KernelRouterAdvertisement = 2,
    /// A link-local address configured by the kernel.
    #[expect(dead_code)]
    KernelLinkLocal = 3,
}

#[derive(Debug)]
pub enum AddrAttr {
    Address(IpAddr),
    Broadcast(Ipv4Addr),
    Flags(AddrMessageFlags),
    Label(CString),
    Local(IpAddr),
    Protocol(AddrProtocol),
}

impl AddrAttr {
    fn class(&self) -> AddrAttrClass {
        match self {
            AddrAttr::Address(_) => AddrAttrClass::ADDRESS,
            AddrAttr::Broadcast(_) => AddrAttrClass::BROADCAST,
            AddrAttr::Flags(_) => AddrAttrClass::FLAGS,
            AddrAttr::Label(_) => AddrAttrClass::LABEL,
            AddrAttr::Local(_) => AddrAttrClass::LOCAL,
            AddrAttr::Protocol(_) => AddrAttrClass::PROTO,
        }
    }
}

impl Attribute for AddrAttr {
    fn type_(&self) -> u16 {
        self.class() as u16
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match self {
            AddrAttr::Address(address) => address.as_octets(),
            AddrAttr::Broadcast(address) => address.as_octets(),
            AddrAttr::Flags(flags) => flags.as_bytes(),
            AddrAttr::Label(label) => label.as_bytes_with_nul(),
            AddrAttr::Local(address) => address.as_octets(),
            AddrAttr::Protocol(protocol) => protocol.as_bytes(),
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
