// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![forbid(unsafe_code)]
#![feature(trait_alias)]
#![feature(fn_traits)]
#![feature(linked_list_cursors)]

mod buffer;
mod dma_pool;
mod driver;

extern crate alloc;

use alloc::{
    collections::{BTreeMap, VecDeque},
    string::String,
    sync::Arc,
    vec::Vec,
};
use core::{any::Any, fmt::Debug};

use aster_frame::{
    sync::{RwLock, SpinLock},
    vm::VmReader,
};
use aster_util::safe_ptr::Pod;
pub use buffer::{RxBuffer, TxBuffer};
use component::{init_component, ComponentInitError};
pub use dma_pool::{DmaPool, DmaSegment};
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

    /// Returns whether the device has packet available for receiving
    fn can_receive(&self) -> bool;

    /// Returns whether the device can send another packet
    fn can_send(&self) -> bool;

    /// Receive a packet from network. If packet is ready, returns a RxBuffer containing the packet.
    /// Otherwise, return NotReady error.
    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError>;

    /// Send a packet to the network.
    ///
    /// Before calling this method,
    /// the user should always use `can_send` to check the device status
    /// and only call this method when `can_send` returns true.
    ///
    /// Additionally, ensure that the packet size does not exceed the MTU length,
    /// which is currently set to 1536 bytes.
    fn send(&mut self, packet: &mut VmReader) -> Result<(), VirtioNetError>;

    /// Send multiple tx buffers to the device.
    ///
    /// This method should only send buffers freed by `free_processed_tx_buffers`.
    /// If you want to send packet with new tx buffer, use `send` method instead.
    fn send_buffers(&mut self, buffers: VecDeque<TxBuffer>) -> Result<(), VirtioNetError>;

    /// Free processed tx buffers.
    ///
    /// Returns the processed buffers for reuse.
    fn free_processed_tx_buffers(&mut self) -> Vec<TxBuffer>;
}

pub trait NetDeviceIrqHandler = Fn() + Send + Sync + 'static;

pub fn register_device(name: String, device: NetworkDeviceRef) {
    COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .write_irq_disabled()
        .insert(name, NetworkDeviceIrqCallbackSet::new(device));
}

pub fn get_device(str: &str) -> Option<NetworkDeviceRef> {
    let device_table = COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .read_irq_disabled();
    let callback_set = device_table.get(str)?;
    Some(callback_set.device.clone())
}

/// Registers callback which will be called when receiving message.
///
/// Since the callback will be called in interrupt context,
/// the callback function should NOT sleep.
pub fn register_recv_callback(name: &str, callback: impl NetDeviceIrqHandler) {
    let device_table = COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .read_irq_disabled();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.recv_callbacks.write().push(Arc::new(callback));
}

/// Registers callback which will be called when sending message are finished.
///
/// Since the callback will be called in interrupt context,
/// the callback function should NOT sleep.
pub fn register_send_callback(name: &str, callback: impl NetDeviceIrqHandler) {
    let device_table = COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .read_irq_disabled();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.send_callbacks.write().push(Arc::new(callback));
}

pub fn handle_recv_irq(name: &str) {
    let recv_callbacks = {
        let device_table = COMPONENT
            .get()
            .unwrap()
            .network_device_table
            .read_irq_disabled();
        if let Some(callback_set) = device_table.get(name) {
            callback_set.recv_callbacks.clone()
        } else {
            return;
        }
    };

    for callback in recv_callbacks.read_irq_disabled().iter() {
        callback();
    }
}

pub fn handle_send_irq(name: &str) {
    let send_callbacks = {
        let device_table = COMPONENT
            .get()
            .unwrap()
            .network_device_table
            .read_irq_disabled();
        if let Some(callback_set) = device_table.get(name) {
            callback_set.send_callbacks.clone()
        } else {
            return;
        }
    };

    for callback in send_callbacks.read_irq_disabled().iter() {
        callback();
    }
}

pub fn all_devices() -> Vec<(String, NetworkDeviceRef)> {
    let network_devs = COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .read_irq_disabled();
    network_devs
        .iter()
        .map(|(name, callback_set)| (name.clone(), callback_set.device.clone()))
        .collect()
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    buffer::init();
    COMPONENT.call_once(|| component);
    Ok(())
}

type NetDeviceIrqHandlerListRef = Arc<RwLock<Vec<Arc<dyn NetDeviceIrqHandler>>>>;
type NetworkDeviceRef = Arc<SpinLock<dyn AnyNetworkDevice>>;

struct Component {
    /// Device list, the key is device name, value is (callbacks, device);
    network_device_table: RwLock<BTreeMap<String, NetworkDeviceIrqCallbackSet>>,
}

struct NetworkDeviceIrqCallbackSet {
    device: NetworkDeviceRef,
    recv_callbacks: NetDeviceIrqHandlerListRef,
    send_callbacks: NetDeviceIrqHandlerListRef,
}

impl NetworkDeviceIrqCallbackSet {
    fn new(device: NetworkDeviceRef) -> Self {
        Self {
            device,
            recv_callbacks: Arc::new(RwLock::new(Vec::new())),
            send_callbacks: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            network_device_table: RwLock::new(BTreeMap::new()),
        })
    }
}
