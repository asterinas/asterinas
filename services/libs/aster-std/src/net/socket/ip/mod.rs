use crate::net::iface::{IpAddress, IpEndpoint};

mod common;
mod datagram;
mod stream;

pub use datagram::DatagramSocket;
pub use stream::StreamSocket;

// According to the Linux man pages and the Linux implementation, `getsockname()` will *not* fail
// even if the socket is unbound. Instead, it will return an unspecified socket address. This dummy
// endpoint helps with that.
const DUMMY_LOCAL_ENDPOINT: IpEndpoint = IpEndpoint::new(IpAddress::v4(0, 0, 0, 0), 0);
