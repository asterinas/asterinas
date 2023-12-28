mod addr;
mod options;
mod socket;

pub use addr::{read_socket_addr_from_user, write_socket_addr_to_user, CSocketAddrFamily};
pub use options::{new_raw_socket_option, CSocketOptionLevel};
pub use socket::{Protocol, SockFlags, SockType, SOCK_TYPE_MASK};

use crate::fs::file_table::FileDescripter;
use crate::net::socket::Socket;
use crate::prelude::*;

pub fn get_socket_from_fd(sockfd: FileDescripter) -> Result<Arc<dyn Socket>> {
    let current = current!();
    let file_table = current.file_table().lock();
    file_table.get_socket(sockfd)
}
