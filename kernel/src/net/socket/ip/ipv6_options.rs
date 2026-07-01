// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::socket::NeedIfacePoll;

use super::options::Ipv6V6only;
use crate::{
    net::socket::options::{
        SocketOption,
        macros::{sock_option_mut, sock_option_ref},
    },
    prelude::*,
};

/// IPv6-level socket options.
#[derive(Clone, Copy, CopyGetters, Debug, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub(super) struct Ipv6OptionSet {
    v6only: bool,
}

impl Ipv6OptionSet {
    pub(super) const fn new() -> Self {
        Self { v6only: false }
    }

    pub(super) fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        sock_option_mut!(match option {
            ipv6_v6only @ Ipv6V6only => {
                let v6only = self.v6only();
                ipv6_v6only.set(v6only);
            }
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option is unknown"),
        });

        Ok(())
    }

    pub(super) fn set_option(&mut self, option: &dyn SocketOption) -> Result<NeedIfacePoll> {
        sock_option_ref!(match option {
            ipv6_v6only @ Ipv6V6only => {
                let v6only = ipv6_v6only.get().unwrap();
                self.set_v6only(*v6only);
            }
            _ => return_errno_with_message!(
                Errno::ENOPROTOOPT,
                "the socket option to be set is unknown"
            ),
        });

        Ok(NeedIfacePoll::FALSE)
    }
}
