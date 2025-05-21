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
use aster_softirq::{
    softirq_id::{NETWORK_RX_SOFTIRQ_ID, NETWORK_TX_SOFTIRQ_ID},
    BottomHalfDisabled, SoftIrqLine,
};
pub use buffer::{RxBuffer, TxBuffer, RX_BUFFER_POOL, TX_BUFFER_LEN};
use component::{init_component, ComponentInitError};
pub use dma_pool::DmaSegment;
use ostd::{sync::SpinLock, Pod};
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

    /// Receives a packet from network. If packet is ready, returns a `RxBuffer` containing the packet.
    /// Otherwise, return [`VirtioNetError::NotReady`].
    fn receive(&mut self) -> Result<RxBuffer, VirtioNetError>;

    /// Sends a packet to network.
    fn send(&mut self, packet: &[u8]) -> Result<(), VirtioNetError>;

    /// Frees processes tx buffers.
    fn free_processed_tx_buffers(&mut self);

    /// Notifies the device driver that a polling operation has ended.
    ///
    /// The driver can assume that the device remains protected by acquiring a poll lock
    /// for the entire duration of the polling process.
    /// Thus two polling process cannot happen simultaneously.
    fn notify_poll_end(&mut self);
}

pub trait NetDeviceCallback = Fn() + Send + Sync + 'static;

pub fn register_device(
    name: String,
    device: Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>,
) {
    COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .lock()
        .insert(name, NetworkDeviceIrqCallbackSet::new(device));
}

pub fn get_device(str: &str) -> Option<Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>> {
    let table = COMPONENT.get().unwrap().network_device_table.lock();
    let callbacks = table.get(str)?;
    Some(callbacks.device.clone())
}

/// Registers callback which will be called when receiving message.
///
/// Since the callback will be called in softirq context,
/// the callback function should _not_ sleep.
pub fn register_recv_callback(name: &str, callback: impl NetDeviceCallback) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.recv_callbacks.lock().push(Arc::new(callback));
}

/// Registers a callback that will be invoked
/// when the device has completed sending a packet.
///
/// Since this callback is executed in a softirq context,
/// the callback function should _not_ block or sleep.
///
/// Please note that the callback may not be called every time a packet is sent.
/// The driver may skip certain callbacks for performance optimization.
pub fn register_send_callback(name: &str, callback: impl NetDeviceCallback) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.send_callbacks.lock().push(Arc::new(callback));
}

fn handle_rx_softirq() {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    // TODO: We should handle network events for just one device per softirq,
    // rather than processing events for all devices.
    // This issue should be addressed once new network devices are added.
    for callback_set in device_table.values() {
        let recv_callbacks = callback_set.recv_callbacks.lock();
        for callback in recv_callbacks.iter() {
            callback();
        }
    }
}

fn handle_tx_softirq() {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    // TODO: We should handle network events for just one device per softirq,
    // rather than processing events for all devices.
    // This issue should be addressed once new network devices are added.
    for callback_set in device_table.values() {
        let can_send = {
            let mut device = callback_set.device.lock();
            device.free_processed_tx_buffers();
            device.can_send()
        };

        if !can_send {
            continue;
        }

        let send_callbacks = callback_set.send_callbacks.lock();
        for callback in send_callbacks.iter() {
            callback();
        }
    }
}

/// Raises softirq for handling transmission events
pub fn raise_send_softirq() {
    SoftIrqLine::get(NETWORK_TX_SOFTIRQ_ID).raise();
}

/// Raises softirq for handling reception events
pub fn raise_receive_softirq() {
    SoftIrqLine::get(NETWORK_RX_SOFTIRQ_ID).raise();
}

pub fn all_devices() -> Vec<(String, NetworkDeviceRef)> {
    let network_devs = COMPONENT.get().unwrap().network_device_table.lock();
    network_devs
        .iter()
        .map(|(name, callbacks)| (name.clone(), callbacks.device.clone()))
        .collect()
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let a = Component::init()?;
    COMPONENT.call_once(|| a);
    SoftIrqLine::get(NETWORK_TX_SOFTIRQ_ID).enable(handle_tx_softirq);
    SoftIrqLine::get(NETWORK_RX_SOFTIRQ_ID).enable(handle_rx_softirq);
    buffer::init();
    Ok(())
}

type NetDeviceCallbackListRef = Arc<SpinLock<Vec<Arc<dyn NetDeviceCallback>>, BottomHalfDisabled>>;
type NetworkDeviceRef = Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>;

struct Component {
    /// Device list, the key is device name, value is (callbacks, device);
    network_device_table:
        SpinLock<BTreeMap<String, NetworkDeviceIrqCallbackSet>, BottomHalfDisabled>,
}

/// The send callbacks and recv callbacks for a network device
struct NetworkDeviceIrqCallbackSet {
    device: NetworkDeviceRef,
    recv_callbacks: NetDeviceCallbackListRef,
    send_callbacks: NetDeviceCallbackListRef,
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
