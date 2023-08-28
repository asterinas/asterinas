#![no_std]
#![forbid(unsafe_code)]
#![feature(trait_alias)]
#![feature(fn_traits)]

pub mod buffer;
pub mod driver;

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use buffer::RxBuffer;
use buffer::TxBuffer;
use component::init_component;
use component::ComponentInitError;
use core::any::Any;
use core::fmt::Debug;
use jinux_frame::sync::SpinLock;
use jinux_util::safe_ptr::Pod;
use smoltcp::phy;
use spin::Once;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct EthernetAddr(pub [u8; 6]);

#[derive(Debug, Clone, Copy)]
pub enum VirtioNetError {
    NotReady,
    WrongToken,
    Unknown,
}

pub trait NetworkDevice: Send + Sync + Any + Debug {
    // ================Device Information=================

    fn mac_addr(&self) -> EthernetAddr;
    fn capabilities(&self) -> phy::DeviceCapabilities;

    // ================Device Operation===================

    fn can_receive(&self) -> bool;
    fn can_send(&self) -> bool;
    /// Receive a packet from network. If packet is ready, returns a RxBuffer containing the packet.
    /// Otherwise, return NotReady error.
    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError>;
    /// Send a packet to network. Return until the request completes.
    fn send(&mut self, tx_buffer: TxBuffer) -> Result<(), VirtioNetError>;
}

pub trait NetDeviceIrqHandler = Fn() + Send + Sync + 'static;

pub fn register_device(name: String, device: Arc<SpinLock<Box<dyn NetworkDevice>>>) {
    COMPONENT
        .get()
        .unwrap()
        .devices
        .lock()
        .insert(name, (Arc::new(SpinLock::new(Vec::new())), device));
}

pub fn get_device(str: &String) -> Option<Arc<SpinLock<Box<dyn NetworkDevice>>>> {
    let lock = COMPONENT.get().unwrap().devices.lock();
    let Some((_, device)) = lock.get(str) else {
        return None;
    };
    Some(device.clone())
}

pub fn register_recv_callback(name: &String, callback: impl NetDeviceIrqHandler) {
    let lock = COMPONENT.get().unwrap().devices.lock();
    let Some((callbacks, _)) = lock.get(name) else {
        return;
    };
    callbacks.lock().push(Arc::new(callback));
}

pub fn handle_recv_irq(name: &String) {
    let lock = COMPONENT.get().unwrap().devices.lock();
    let Some((callbacks, _)) = lock.get(name) else {
        return;
    };
    let callbacks = callbacks.clone();
    let lock = callbacks.lock();
    for callback in lock.iter() {
        callback.call(())
    }
}

pub fn all_devices() -> Vec<(String, Arc<SpinLock<Box<dyn NetworkDevice>>>)> {
    let lock = COMPONENT.get().unwrap().devices.lock();
    let mut vec = Vec::new();
    for (name, (_, device)) in lock.iter() {
        vec.push((name.clone(), device.clone()));
    }
    vec
}

static COMPONENT: Once<Component> = Once::new();
pub(crate) static NETWORK_IRQ_HANDLERS: Once<SpinLock<Vec<Arc<dyn NetDeviceIrqHandler>>>> =
    Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let a = Component::init()?;
    COMPONENT.call_once(|| a);
    NETWORK_IRQ_HANDLERS.call_once(|| SpinLock::new(Vec::new()));
    Ok(())
}

struct Component {
    /// Device list, the key is device name, value is (callbacks, device);
    devices: SpinLock<
        BTreeMap<
            String,
            (
                Arc<SpinLock<Vec<Arc<dyn NetDeviceIrqHandler>>>>,
                Arc<SpinLock<Box<dyn NetworkDevice>>>,
            ),
        >,
    >,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            devices: SpinLock::new(BTreeMap::new()),
        })
    }
}
