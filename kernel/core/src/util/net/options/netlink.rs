// SPDX-License-Identifier: MPL-2.0

use super::{RawSocketOption, SocketOption, impl_raw_sock_option_set_only, impl_raw_socket_option};
use crate::{
    net::socket::netlink::{AddMembership, DropMembership, NetlinkGetStrictChk},
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
        CNetlinkOptionName::GET_STRICT_CHK => Ok(Box::new(NetlinkGetStrictChk::new())),
        _ => return_errno_with_message!(Errno::ENOPROTOOPT, "unsupported netlink option"),
    }
}

impl_raw_sock_option_set_only!(AddMembership);
impl_raw_sock_option_set_only!(DropMembership);

impl_raw_socket_option!(NetlinkGetStrictChk);
