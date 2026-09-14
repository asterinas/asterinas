// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU8;

use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;

use super::RawSocketOption;
use crate::{
    context::current_userspace,
    net::socket::ip::options::{Hdrincl, IpTtl, Recverr, Tos, Ttl},
    prelude::*,
};

/// Socket options for IP socket.
///
/// The raw definitions can be found at:
/// <https://elixir.bootlin.com/linux/v6.0.19/source/include/uapi/linux/in.h#L94>.
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum CIpOptionName {
    TOS = 1,
    TTL = 2,
    HDRINCL = 3,
    OPTIONS = 4,
    ROUTER_ALERT = 5,
    RECVOPTS = 6,
    RETOPTS = 7,
    PKTINFO = 8,
    PKTOPTIONS = 9,
    MTU_DISCOVER = 10,
    RECVERR = 11,
    RECVTTL = 12,
    RECVTOS = 13,
    MTU = 14,
    FREEBIND = 15,
    IPSEC_POLICY = 16,
    XFRM_POLICY = 17,
    PASSSEC = 18,
    TRANSPARENT = 19,
    ORIGDSTADDR = 20,
    MINTTL = 21,
    NODEFRAG = 22,
    CHECKSUM = 23,
    BIND_ADDRESS_NO_PORT = 24,
    RECVFRAGSIZE = 25,
    RECVERR_RFC4884 = 26,
    MULTICAST_IF = 32,
    MULTICAST_TTL = 33,
    MULTICAST_LOOP = 34,
    ADD_MEMBERSHIP = 35,
    DROP_MEMBERSHIP = 36,
    UNBLOCK_SOURCE = 37,
    BLOCK_SOURCE = 38,
    ADD_SOURCE_MEMBERSHIP = 39,
    DROP_SOURCE_MEMBERSHIP = 40,
    MSFILTER = 41,
    MCAST_JOIN_GROUP = 42,
    MCAST_BLOCK_SOURCE = 43,
    MCAST_UNBLOCK_SOURCE = 44,
    MCAST_LEAVE_GROUP = 45,
    MCAST_JOIN_SOURCE_GROUP = 46,
    MCAST_LEAVE_SOURCE_GROUP = 47,
    MCAST_MSFILTER = 48,
    MULTICAST_ALL = 49,
    UNICAST_IF = 50,
}

pub(crate) fn new_ip_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CIpOptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CIpOptionName::TOS => Ok(Box::new(Tos::new())),
        CIpOptionName::TTL => Ok(Box::new(Ttl::new())),
        CIpOptionName::HDRINCL => Ok(Box::new(Hdrincl::new())),
        CIpOptionName::RECVERR => Ok(Box::new(Recverr::new())),
        _ => return_errno_with_message!(Errno::ENOPROTOOPT, "unsupported ip level option"),
    }
}

trait ReadIpOption: Sized {
    fn read_ip_option(addr: Vaddr, max_len: u32) -> Result<Self>;
}

macro_rules! impl_raw_ip_socket_option {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()> {
                let input = ReadIpOption::read_ip_option(addr, max_len)?;
                self.set(input);
                Ok(())
            }

            fn write_to_user(&self, addr: Vaddr, max_len: &mut u32) -> Result<usize> {
                use $crate::util::net::options::utils::WriteToUser;

                let output = self.get().unwrap();
                output.write_to_user(addr, *max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn super::SocketOption {
                self
            }

            fn as_sock_option(&self) -> &dyn super::SocketOption {
                self
            }
        }
    };
}

impl_raw_ip_socket_option!(Ttl);
impl_raw_ip_socket_option!(Tos);
impl_raw_ip_socket_option!(Hdrincl);
impl_raw_ip_socket_option!(Recverr);

impl ReadIpOption for i32 {
    fn read_ip_option(addr: Vaddr, max_len: u32) -> Result<Self> {
        read_ip_int(addr, max_len)
    }
}

impl ReadIpOption for bool {
    fn read_ip_option(addr: Vaddr, max_len: u32) -> Result<Self> {
        Ok(read_ip_int(addr, max_len)? != 0)
    }
}

impl ReadIpOption for IpTtl {
    fn read_ip_option(addr: Vaddr, max_len: u32) -> Result<Self> {
        let val = read_ip_int(addr, max_len)?;

        let ttl_value = match val {
            -1 => None,
            1..=255 => Some(NonZeroU8::new(val as u8).unwrap()),
            _ => return_errno_with_message!(Errno::EINVAL, "invalid ttl value"),
        };

        Ok(IpTtl::new(ttl_value))
    }
}

// Reference: <https://elixir.bootlin.com/linux/v7.2.5/source/net/ipv4/ip_sockglue.c#L927-L936>.
fn read_ip_int(addr: Vaddr, max_len: u32) -> Result<i32> {
    if max_len >= size_of::<i32>() as u32 {
        Ok(current_userspace!().read_val::<i32>(addr)?)
    } else if max_len >= 1 {
        Ok(current_userspace!().read_val::<u8>(addr)? as i32)
    } else {
        Ok(0)
    }
}
