// SPDX-License-Identifier: MPL-2.0

mod addr;
mod ns;
mod stream;

pub use addr::UnixSocketAddr;
pub use stream::UnixStreamSocket;
pub(super) use stream::UNIX_STREAM_DEFAULT_BUF_SIZE;
