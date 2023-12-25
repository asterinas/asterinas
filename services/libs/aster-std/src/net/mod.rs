use crate::{
    net::iface::{Iface, IfaceLoopback, IfaceVirtio},
    prelude::*,
};
use spin::Once;

use self::iface::spawn_background_poll_thread;

pub static IFACES: Once<Vec<Arc<dyn Iface>>> = Once::new();

pub mod iface;
pub mod socket;

pub fn init() {
    IFACES.call_once(|| {
        let iface_virtio = IfaceVirtio::new();
        let iface_loopback = IfaceLoopback::new();
        vec![iface_virtio, iface_loopback]
    });

    for (name, _) in aster_network::all_devices() {
        aster_network::register_recv_callback(&name, || {
            // TODO: further check that the irq num is the same as iface's irq num
            let iface_virtio = &IFACES.get().unwrap()[0];
            iface_virtio.poll();
        })
    }
    poll_ifaces();
}

/// Lazy init should be called after spawning init thread.
pub fn lazy_init() {
    for iface in IFACES.get().unwrap() {
        spawn_background_poll_thread(iface.clone());
    }
}

/// Poll iface
pub fn poll_ifaces() {
    let ifaces = IFACES.get().unwrap();
    for iface in ifaces.iter() {
        iface.poll();
    }
}
