// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]
#![feature(trait_alias)]
#![feature(fn_traits)]
#![feature(linked_list_cursors)]

mod buffer;
pub mod dma_pool;
mod driver;

extern crate alloc;

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::{any::Any, fmt::Debug};

use aster_bigtcp::device::DeviceCapabilities;
pub use buffer::{RxBuffer, TxBuffer, RX_BUFFER_POOL, TX_BUFFER_LEN};
use component::{init_component, ComponentInitError};
pub use dma_pool::DmaSegment;
use ostd::{
    sync::{LocalIrqDisabled, SpinLock},
    Pod,
};
use spin::Once;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct EthernetAddr(pub [u8; 6]);

#[derive(Debug, Clone, Copy)]
pub enum VirtioNetError {
    NotReady,
    WrongToken,
    Busy,
    Unknown,
}

pub trait AnyNetworkDevice: Send + Sync + Any + Debug {
    // ================Device Information=================

    fn mac_addr(&self) -> EthernetAddr;
    fn capabilities(&self) -> DeviceCapabilities;

    // ================Device Operation===================

    fn can_receive(&self) -> bool;
    fn can_send(&self) -> bool;
    /// Receive a packet from network. If packet is ready, returns a RxBuffer containing the packet.
    /// Otherwise, return NotReady error.
    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError>;
    /// Send a packet to network. Return until the request completes.
    fn send(&mut self, packet: &[u8]) -> Result<(), VirtioNetError>;
    fn free_processed_tx_buffers(&mut self);
}

pub trait NetDeviceIrqHandler = Fn() + Send + Sync + 'static;

pub fn register_device(
    name: String,
    device: Arc<SpinLock<dyn AnyNetworkDevice, LocalIrqDisabled>>,
) {
    COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .lock()
        .insert(name, NetworkDeviceIrqCallbackSet::new(device));
}

pub fn get_device(str: &str) -> Option<Arc<SpinLock<dyn AnyNetworkDevice, LocalIrqDisabled>>> {
    let table = COMPONENT.get().unwrap().network_device_table.lock();
    let callbacks = table.get(str)?;
    Some(callbacks.device.clone())
}

/// Registers callback which will be called when receiving message.
///
/// Since the callback will be called in interrupt context,
/// the callback function should NOT sleep.
pub fn register_recv_callback(name: &str, callback: impl NetDeviceIrqHandler) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.recv_callbacks.lock().push(Arc::new(callback));
}

pub fn register_send_callback(name: &str, callback: impl NetDeviceIrqHandler) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.send_callbacks.lock().push(Arc::new(callback));
}

pub fn handle_recv_irq(name: &str) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };

    let callbacks = callbacks.recv_callbacks.lock();
    for callback in callbacks.iter() {
        callback();
    }
}

pub fn handle_send_irq(name: &str) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };

    let can_send = {
        let mut device = callbacks.device.lock();
        device.free_processed_tx_buffers();
        device.can_send()
    };
    if !can_send {
        return;
    }

    let callbacks = callbacks.send_callbacks.lock();
    for callback in callbacks.iter() {
        callback();
    }
}

pub fn all_devices() -> Vec<(String, NetworkDeviceRef)> {
    let network_devs = COMPONENT.get().unwrap().network_device_table.lock();
    network_devs
        .iter()
        .map(|(name, callbacks)| (name.clone(), callbacks.device.clone()))
        .collect()
}

static COMPONENT: Once<Component> = Once::new();
pub(crate) static NETWORK_IRQ_HANDLERS: Once<
    SpinLock<Vec<Arc<dyn NetDeviceIrqHandler>>, LocalIrqDisabled>,
> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let a = Component::init()?;
    COMPONENT.call_once(|| a);
    NETWORK_IRQ_HANDLERS.call_once(|| SpinLock::new(Vec::new()));
    buffer::init();
    Ok(())
}

type NetDeviceIrqHandlerListRef =
    Arc<SpinLock<Vec<Arc<dyn NetDeviceIrqHandler>>, LocalIrqDisabled>>;
type NetworkDeviceRef = Arc<SpinLock<dyn AnyNetworkDevice, LocalIrqDisabled>>;

struct Component {
    /// Device list, the key is device name, value is (callbacks, device);
    network_device_table: SpinLock<BTreeMap<String, NetworkDeviceIrqCallbackSet>, LocalIrqDisabled>,
}

/// The send callbacks and recv callbacks for a network device
struct NetworkDeviceIrqCallbackSet {
    device: NetworkDeviceRef,
    recv_callbacks: NetDeviceIrqHandlerListRef,
    send_callbacks: NetDeviceIrqHandlerListRef,
}

impl NetworkDeviceIrqCallbackSet {
    fn new(device: NetworkDeviceRef) -> Self {
        Self {
            device,
            recv_callbacks: Arc::new(SpinLock::new(Vec::new())),
            send_callbacks: Arc::new(SpinLock::new(Vec::new())),
        }
    }
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            network_device_table: SpinLock::new(BTreeMap::new()),
        })
    }
}
