// SPDX-License-Identifier: MPL-2.0

//! The virtio-vsock transport protocol.
//!
//! Built on top of the [_basic virtio-vsock device support_](`aster_virtio::device::socket`), this
//! module manages connections and listeners, handles connection establishment, data transfer, and
//! shutdown, and implements credit-based flow control plus I/O event checking and notification.
//! The socket layer is expected to build on these APIs to provide the user-visible socket
//! interface.
//!
//! For a quick start, bind to a port by creating a [`BoundPort`] instance.
//!  - To connect to a remote address, use the [`BoundPort::connect`] method and get a
//!    [`Connection`] instance. Data can be transmitted or received via the [`Connection::try_send`]
//!    and [`Connection::try_recv`] methods.
//!  - To listen to the local address, use the [`BoundPort::listen`] method and get a [`Listener`]
//!    instance. Incoming connections can be accepted via the [`Listener::try_accept`] method.
//!
//! Drop the [`BoundPort`], [`Connection`], or [`Listener`] instance will shut down the underlying
//! connection (if any) and release the resources.

mod conn_id;
mod connection;
mod listener;
mod port;
mod space;
mod timer;

use core::time::Duration;

pub(super) use connection::{Connection, connect::ConnectResult};
pub(super) use listener::Listener;
pub(super) use port::BoundPort;

// Reference: <https://elixir.bootlin.com/linux/v6.16.8/source/net/vmw_vsock/af_vsock.c#L136>
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
// Reference: <https://elixir.bootlin.com/linux/v6.16.8/source/net/vmw_vsock/virtio_transport_common.c#L24>
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(8);
// Reference: <https://elixir.bootlin.com/linux/v6.16.8/source/net/vmw_vsock/af_vsock.c#L138>
const DEFAULT_RX_BUF_SIZE: usize = 256 * 1024;
// Reference: <https://elixir.bootlin.com/linux/v6.16.8/source/net/vmw_vsock/af_vsock.c#L138>
const DEFAULT_TX_BUF_SIZE: usize = 256 * 1024;
// Reference: <https://elixir.bootlin.com/linux/v6.16.8/source/include/linux/socket.h#L298>
const MAX_BACKLOG: usize = 4096;

const CREDIT_UPDATE_THRESHOLD: u32 = (DEFAULT_RX_BUF_SIZE / 4) as u32;

fn process_rx_callback() {
    if let Ok(vsock_space) = space::vsock_space() {
        vsock_space.process_rx();
    }
}

fn process_event_callback() {
    if let Ok(vsock_space) = space::vsock_space() {
        vsock_space.process_transport_event();
    }
}

/// Initializes the virtio-vsock transport when the default device is present.
pub(super) fn init() {
    use aster_virtio::device::socket::DEVICE_NAME;

    let Some(device) = aster_virtio::device::socket::get_device(DEVICE_NAME) else {
        return;
    };

    device.init_rx_callback(process_rx_callback);
    device.init_event_callback(process_event_callback);
    space::init(device);

    timer::init();
}
