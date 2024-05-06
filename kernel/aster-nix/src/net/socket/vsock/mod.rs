// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_virtio::device::socket::{get_device, register_recv_callback, DEVICE_NAME};
use common::VsockSpace;
use spin::Once;

pub mod addr;
pub mod common;
pub mod stream;
pub use addr::VsockSocketAddr;
pub use stream::VsockStreamSocket;

// init static driver
pub static VSOCK_GLOBAL: Once<Arc<VsockSpace>> = Once::new();

pub fn init() {
    if get_device(DEVICE_NAME).is_some() {
        VSOCK_GLOBAL.call_once(|| Arc::new(VsockSpace::new()));
        register_recv_callback(DEVICE_NAME, || {
            let vsockspace = VSOCK_GLOBAL.get().unwrap();
            vsockspace.poll().unwrap();
        })
    }
}
