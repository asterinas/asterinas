// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroU8;

use aster_bigtcp::socket::NeedIfacePoll;

use crate::{
    impl_socket_options, match_sock_option_mut, match_sock_option_ref,
    net::socket::options::SocketOption, prelude::*,
};

/// IP-level socket options.
#[derive(Debug, Clone, Copy, CopyGetters, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub(super) struct IpOptionSet {
    tos: u8,
    ttl: IpTtl,
    hdrincl: bool,
}

const DEFAULT_TTL: u8 = 64;
pub(super) const INET_ECN_MASK: u8 = 3;

impl IpOptionSet {
    pub(super) const fn new_tcp() -> Self {
        Self {
            tos: 0,
            ttl: IpTtl(None),
            hdrincl: false,
        }
    }

    pub(super) fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        match_sock_option_mut!(option, {
            ip_tos: Tos => {
                let tos = self.tos();
                ip_tos.set(tos as _);
            },
            ip_ttl: Ttl => {
                let ttl = self.ttl();
                ip_ttl.set(ttl);
            },
            ip_hdrincl: Hdrincl => {
                let hdrincl = self.hdrincl();
                ip_hdrincl.set(hdrincl);
            },
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option is unknown")
        });

        Ok(())
    }

    pub(super) fn set_option(
        &mut self,
        option: &dyn SocketOption,
        socket: &dyn SetIpLevelOption,
    ) -> Result<NeedIfacePoll> {
        match_sock_option_ref!(option, {
            ip_tos: Tos => {
                let old_value = self.tos();
                let mut val = *ip_tos.get().unwrap() as u8;
                val &= !INET_ECN_MASK;
                val |= old_value & INET_ECN_MASK;
                self.set_tos(val);
            },
            ip_ttl: Ttl => {
                let ttl = ip_ttl.get().unwrap();
                self.set_ttl(*ttl);
            },
            ip_hdrincl: Hdrincl => {
                let hdrincl = ip_hdrincl.get().unwrap();
                socket.set_hdrincl(*hdrincl)?;
                self.set_hdrincl(*hdrincl);
            },
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option to be set is unknown")
        });

        Ok(NeedIfacePoll::FALSE)
    }
}

impl_socket_options!(
    pub struct Tos(i32);
    pub struct Ttl(IpTtl);
    pub struct Hdrincl(bool);
);

#[derive(Debug, Clone, Copy)]
pub struct IpTtl(Option<NonZeroU8>);

impl IpTtl {
    pub const fn new(val: Option<NonZeroU8>) -> Self {
        Self(val)
    }

    pub const fn get(&self) -> u8 {
        if let Some(val) = self.0 {
            val.get()
        } else {
            DEFAULT_TTL
        }
    }
}

pub(super) trait SetIpLevelOption {
    fn set_hdrincl(&self, _hdrincl: bool) -> Result<()>;
}
