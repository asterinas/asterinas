// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::options::{
        SocketOption,
        macros::{impl_socket_options, sock_option_mut},
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NetlinkOptionSet {
    strict_check: bool,
}

impl NetlinkOptionSet {
    pub(super) const fn new() -> Self {
        Self {
            strict_check: false,
        }
    }

    pub(super) const fn strict_check(&self) -> bool {
        self.strict_check
    }

    pub(super) fn set_strict_check(&mut self, strict_check: bool) {
        self.strict_check = strict_check;
    }

    pub(super) fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        sock_option_mut!(match option {
            strict @ StrictCheck => {
                strict.set(self.strict_check);
            }
            _ => return_errno_with_message!(Errno::ENOPROTOOPT, "the socket option is unknown"),
        });
        Ok(())
    }
}

impl_socket_options!(
    pub(crate) struct AddMembership(u32);
    pub(crate) struct DropMembership(u32);
    pub(crate) struct StrictCheck(bool);
);
