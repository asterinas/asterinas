// SPDX-License-Identifier: MPL-2.0

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
use aster_frame::sync::SpinLock;
use aster_util::safe_ptr::Pod;
use buffer::RxBuffer;
use buffer::TxBuffer;
use component::init_component;
use component::ComponentInitError;
use core::any::Any;
use core::fmt::Debug;
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

pub trait AnyNetworkDevice: Send + Sync + Any + Debug {
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

pub fn register_device(name: String, device: Arc<SpinLock<Box<dyn AnyNetworkDevice>>>) {
    COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .lock()
        .insert(name, (Arc::new(SpinLock::new(Vec::new())), device));
}

pub fn get_device(str: &str) -> Option<Arc<SpinLock<Box<dyn AnyNetworkDevice>>>> {
    let lock = COMPONENT.get().unwrap().network_device_table.lock();
    let Some((_, device)) = lock.get(str) else {
        return None;
    };
    Some(device.clone())
}

pub fn register_recv_callback(name: &str, callback: impl NetDeviceIrqHandler) {
    let lock = COMPONENT.get().unwrap().network_device_table.lock();
    let Some((callbacks, _)) = lock.get(name) else {
        return;
    };
    callbacks.lock().push(Arc::new(callback));
}

pub fn handle_recv_irq(name: &str) {
    let lock = COMPONENT.get().unwrap().network_device_table.lock();
    let Some((callbacks, _)) = lock.get(name) else {
        return;
    };
    let callbacks = callbacks.clone();
    let lock = callbacks.lock();
    for callback in lock.iter() {
        callback.call(())
    }
}

pub fn all_devices() -> Vec<(String, NetworkDeviceRef)> {
    let network_devs = COMPONENT.get().unwrap().network_device_table.lock();
    network_devs
        .iter()
        .map(|(name, (_, device))| (name.clone(), device.clone()))
        .collect()
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

type NetDeviceIrqHandlerListRef = Arc<SpinLock<Vec<Arc<dyn NetDeviceIrqHandler>>>>;
type NetworkDeviceRef = Arc<SpinLock<Box<dyn AnyNetworkDevice>>>;

struct Component {
    /// Device list, the key is device name, value is (callbacks, device);
    network_device_table:
        SpinLock<BTreeMap<String, (NetDeviceIrqHandlerListRef, NetworkDeviceRef)>>,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            network_device_table: SpinLock::new(BTreeMap::new()),
        })
    }
}
