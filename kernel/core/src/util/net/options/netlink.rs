// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{RawSocketOption, SocketOption};
use crate::{
    context::current_userspace,
    net::socket::netlink::{AddMembership, DropMembership, StrictCheck},
    prelude::*,
};

/// Socket options for netlink socket.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/netlink.h#L149>.
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum CNetlinkOptionName {
    ADD_MEMBERSHIP = 1,
    DROP_MEMBERSHIP = 2,
    PKTINFO = 3,
    GET_STRICT_CHK = 12,
}

pub(crate) fn new_netlink_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CNetlinkOptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CNetlinkOptionName::ADD_MEMBERSHIP => Ok(Box::new(AddMembership::new())),
        CNetlinkOptionName::DROP_MEMBERSHIP => Ok(Box::new(DropMembership::new())),
        CNetlinkOptionName::GET_STRICT_CHK => Ok(Box::new(StrictCheck::new())),
        _ => return_errno_with_message!(Errno::ENOPROTOOPT, "unsupported netlink option"),
    }
}

trait ReadNetlinkOption: Sized {
    fn read_netlink_option(addr: Vaddr, max_len: u32) -> Result<Self>;
}

// Netlink treats short inputs as zero and rejects short output buffers.
// References:
// - <https://elixir.bootlin.com/linux/v7.1/source/net/netlink/af_netlink.c#L1658>.
// - <https://elixir.bootlin.com/linux/v7.1/source/net/netlink/af_netlink.c#L1786>.
macro_rules! impl_raw_netlink_socket_option {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()> {
                let input = ReadNetlinkOption::read_netlink_option(addr, max_len)?;
                self.set(input);
                Ok(())
            }

            fn write_to_user(&self, addr: Vaddr, max_len: &mut u32) -> Result<usize> {
                use $crate::util::net::options::utils::WriteToUser;

                if *max_len < size_of::<u32>() as u32 {
                    return_errno_with_message!(Errno::EINVAL, "max_len is too short");
                }
                let output = self.get().unwrap();
                output.write_to_user(addr, *max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SocketOption {
                self
            }
        }
    };
}

macro_rules! impl_raw_netlink_sock_option_set_only {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()> {
                let input = ReadNetlinkOption::read_netlink_option(addr, max_len)?;
                self.set(input);
                Ok(())
            }

            fn write_to_user(&self, _addr: Vaddr, _max_len: &mut u32) -> Result<usize> {
                return_errno_with_message!(Errno::ENOPROTOOPT, "the option is setter-only");
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SocketOption {
                self
            }
        }
    };
}

impl_raw_netlink_sock_option_set_only!(AddMembership);
impl_raw_netlink_sock_option_set_only!(DropMembership);
impl_raw_netlink_socket_option!(StrictCheck);

impl ReadNetlinkOption for u32 {
    fn read_netlink_option(addr: Vaddr, max_len: u32) -> Result<Self> {
        // Short inputs must not access user memory.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/netlink/af_netlink.c#L1647>.
        if max_len >= size_of::<u32>() as u32 {
            Ok(current_userspace!().read_val::<u32>(addr)?)
        } else {
            Ok(0)
        }
    }
}

impl ReadNetlinkOption for bool {
    fn read_netlink_option(addr: Vaddr, max_len: u32) -> Result<Self> {
        Ok(u32::read_netlink_option(addr, max_len)? != 0)
    }
}
