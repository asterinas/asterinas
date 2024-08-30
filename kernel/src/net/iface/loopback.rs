// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use crossbeam_queue::ArrayQueue;
use smoltcp::{
    iface::Config,
    phy::{self, ChecksumCapabilities, Device, DeviceCapabilities, Medium},
    time::Instant,
    wire::IpCidr,
};
use spin::Once;

use super::{common::IfaceCommon, internal::IfaceInternal, Iface};
use crate::{
    net::{
        iface::time::get_network_timestamp,
        socket::ip::{IpAddress, Ipv4Address},
    },
    prelude::*,
};

pub const LOOPBACK_ADDRESS: IpAddress = {
    let ipv4_addr = Ipv4Address::new(127, 0, 0, 1);
    IpAddress::Ipv4(ipv4_addr)
};
pub const LOOPBACK_ADDRESS_PREFIX_LEN: u8 = 8; // mask: 255.0.0.0

pub struct IfaceLoopback {
    driver: Mutex<Loopback>,
    common: IfaceCommon,
    weak_self: Weak<Self>,
}

impl IfaceLoopback {
    pub fn new() -> Arc<Self> {
        let mut loopback = Loopback::new(Medium::Ip);
        let interface = {
            let config = Config::new(smoltcp::wire::HardwareAddress::Ip);
            let now = get_network_timestamp();

            let mut interface = smoltcp::iface::Interface::new(config, &mut loopback, now);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                let ip_addr = IpCidr::new(LOOPBACK_ADDRESS, LOOPBACK_ADDRESS_PREFIX_LEN);
                ip_addrs.push(ip_addr).unwrap();
            });
            interface
        };
        println!("Loopback ipaddr: {}", interface.ipv4_addr().unwrap());
        let common = IfaceCommon::new(interface);
        Arc::new_cyclic(|weak| Self {
            driver: Mutex::new(loopback),
            common,
            weak_self: weak.clone(),
        })
    }
}

impl IfaceInternal for IfaceLoopback {
    fn common(&self) -> &IfaceCommon {
        &self.common
    }

    fn arc_self(&self) -> Arc<dyn Iface> {
        self.weak_self.upgrade().unwrap()
    }
}

impl Iface for IfaceLoopback {
    fn name(&self) -> &str {
        "lo"
    }

    fn mac_addr(&self) -> Option<smoltcp::wire::EthernetAddress> {
        None
    }

    fn poll(&self) {
        let mut device = self.driver.lock();
        self.common.poll(&mut *device);
    }
}

use alloc::{vec, vec::Vec};

/// A loopback device.
#[derive(Debug)]
pub struct Loopback {
    queue: LinkedList<Packet>,
    medium: Medium,
}

#[allow(clippy::new_without_default)]
impl Loopback {
    /// Creates a loopback device.
    ///
    /// Every packet transmitted through this device will be received through it
    /// in FIFO order.
    pub fn new(medium: Medium) -> Loopback {
        Loopback {
            queue: LinkedList::new(),
            medium,
        }
    }
}

impl Device for Loopback {
    type RxToken<'a> = RxToken;
    type TxToken<'a> = TxToken<'a>;

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.checksum = ChecksumCapabilities::ignored();
        caps.max_transmission_unit = 65535;
        caps.medium = self.medium;

        caps
    }

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.queue.pop_front().map(move |buffer| {
            let rx = RxToken { packet: buffer };
            let tx = TxToken {
                queue: &mut self.queue,
            };
            (rx, tx)
        })
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(TxToken {
            queue: &mut self.queue,
        })
    }
}

#[doc(hidden)]
pub struct RxToken {
    packet: Packet,
}

impl phy::RxToken for RxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let Self { packet } = self;
        let r = f(packet.as_slice());
        packet.recycle();
        r
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct TxToken<'a> {
    queue: &'a mut LinkedList<Packet>,
}

impl<'a> phy::TxToken for TxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut packet = Packet::new(len);
        let result = f(packet.as_mut_slice());
        self.queue.push_back(packet);
        result
    }
}

#[derive(Debug)]
enum Storage {
    Pooled(PooledStorage),
    Global(Vec<u8>),
}

#[derive(Debug)]
struct PooledStorage(Vec<u8>);

impl PooledStorage {
    fn new(size: usize) -> Self {
        let buffer = vec![0u8; size];
        Self(buffer)
    }
}

#[derive(Debug)]
struct Packet {
    storage: Storage,
    len: usize,
}

impl Packet {
    pub fn new(len: usize) -> Self {
        if len <= PAGE_SIZE * 16 {
            let storage_pool = STORAGE_POOL.get().unwrap();
            if let Some(storage) = storage_pool.alloc(len) {
                return Self {
                    storage: Storage::Pooled(storage),
                    len,
                };
            }
        }

        let storage = Storage::Global(vec![0u8; len]);
        Self { storage, len }
    }

    fn as_slice(&self) -> &[u8] {
        match &self.storage {
            Storage::Pooled(storage) => &storage.0[..self.len],
            Storage::Global(storage) => &storage[..self.len],
        }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        match &mut self.storage {
            Storage::Pooled(storage) => &mut storage.0[..self.len],
            Storage::Global(storage) => &mut storage[..self.len],
        }
    }

    fn recycle(self) {
        let Self { storage, .. } = self;
        if let Storage::Pooled(pooled_storage) = storage {
            STORAGE_POOL.get().unwrap().add_storage(pooled_storage);
        }
    }
}

struct StoragePool {
    storage: Vec<ArrayQueue<PooledStorage>>,
}

impl StoragePool {
    pub fn new() -> Self {
        let mut buffers = Vec::with_capacity(16);
        // 4k - 64k
        for i in 1..=16 {
            let size = PAGE_SIZE * i;
            let buffer_num = 64;
            let tx_buffers = ArrayQueue::new(buffer_num);
            for _ in 0..buffer_num {
                let storage = PooledStorage::new(size);
                tx_buffers.push(storage).unwrap();
            }
            buffers.push(tx_buffers);
        }

        Self { storage: buffers }
    }

    pub fn alloc(&self, size: usize) -> Option<PooledStorage> {
        let size = size.align_up(PAGE_SIZE);
        if size > PAGE_SIZE * 16 {
            return None;
        }

        let index = size / PAGE_SIZE - 1;
        self.storage[index].pop()
    }

    pub fn add_storage(&self, storage: PooledStorage) {
        let index = storage.0.len() / PAGE_SIZE - 1;
        self.storage[index].push(storage).unwrap();
    }
}

static STORAGE_POOL: Once<StoragePool> = Once::new();

pub fn init() {
    STORAGE_POOL.call_once(StoragePool::new);
}
