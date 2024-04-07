// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_virtio::device::socket::{register_recv_callback, DEVICE_NAME};
use common::VsockSpace;
use spin::Once;

pub mod addr;
pub mod common;
pub mod stream;
pub use stream::VsockStreamSocket;

// init static driver
pub static VSOCK_GLOBAL: Once<Arc<VsockSpace>> = Once::new();

pub fn init() {
    VSOCK_GLOBAL.call_once(|| Arc::new(VsockSpace::new()));
    register_recv_callback(DEVICE_NAME, || {
        let vsockspace = VSOCK_GLOBAL.get().unwrap();
        let _ = vsockspace.poll();
    })
}
