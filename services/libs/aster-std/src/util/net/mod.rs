// SPDX-License-Identifier: MPL-2.0

mod addr;
mod socket;

pub use addr::{read_socket_addr_from_user, write_socket_addr_to_user, SaFamily};
pub use socket::{Protocol, SockFlags, SockType, SOCK_TYPE_MASK};

#[macro_export]
macro_rules! get_socket_without_holding_filetable_lock {
    ($name:tt, $current: expr, $sockfd: expr) => {
        let file_like = {
            let file_table = $current.file_table().lock();
            file_table.get_file($sockfd)?.clone()
            // Drop filetable here to avoid locking
        };
        let $name = file_like
            .as_socket()
            .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the file is not socket"))?;
    };
}
