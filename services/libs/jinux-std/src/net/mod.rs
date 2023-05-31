use crate::{
    net::iface::{Iface, IfaceLoopback},
    prelude::*,
};
use spin::Once;

use self::iface::spawn_background_poll_thread;

pub static IFACES: Once<Vec<Arc<dyn Iface>>> = Once::new();

pub mod iface;
pub mod socket;

pub fn init() {
    IFACES.call_once(|| {
        let iface_loopback = IfaceLoopback::new();
        vec![iface_loopback]
    });
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
