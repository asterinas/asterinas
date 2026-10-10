// SPDX-License-Identifier: MPL-2.0

//! The ENA network device: one Tx/Rx queue pair, software checksums,
//! MSI-X interrupts plus a timer-tick poll as a fallback.

use alloc::{string::ToString, sync::Arc, vec::Vec};
use core::{
    fmt::Debug,
    sync::atomic::{Ordering, fence},
};

use aster_bigtcp::{
    device::{AnyNetworkDevice, Checksum, DeviceCapabilities, EthernetAddress, Medium, NetError},
    packet::{
        ApplicationLayer, FreshTxPacket, Layer, LinkLayer, RxBuffer, RxPacket, TxBuffer, TxPacket,
    },
};
use aster_pci::capability::msix::CapabilityMsixData;
use dma_pool::DmaPool;
use ostd::{
    arch::trap::TrapFrame,
    mm::{
        HasDaddr, HasSize,
        dma::{FromDevice, ToDevice},
    },
    sync::SpinLock,
};
use spin::Once;

use crate::{
    admin::{self, AdminError, AdminQueue},
    io::{self, CompletionRing, IO_QUEUE_DEPTH, RxCdesc, RxDesc, SubmissionRing, TxCdesc, TxDesc},
};

pub const DEVICE_NAME: &str = "ENA";

const RX_BUFFER_LEN: usize = 4096;
pub(crate) const TX_BUFFER_LEN: usize = 4096;
/// Link-layer MTU: 1500 bytes of IP plus the Ethernet header.
const MTU: usize = 1514;

static RX_POOL: Once<Arc<DmaPool<FromDevice>>> = Once::new();
static TX_POOL: Once<Arc<DmaPool<ToDevice>>> = Once::new();

pub(crate) fn init_pools() {
    RX_POOL.call_once(|| DmaPool::new(RX_BUFFER_LEN, 64, 192, false));
    TX_POOL.call_once(|| DmaPool::new(TX_BUFFER_LEN, 32, 128, false));
}

#[derive(Debug)]
#[expect(dead_code)] // the payloads are for `{:?}` in error messages
pub(crate) enum EnaError {
    Admin(AdminError),
    Alloc(ostd::Error),
    /// The device reports fewer MSI-X vectors than the two we need.
    NoMsix,
}

impl From<AdminError> for EnaError {
    fn from(e: AdminError) -> Self {
        EnaError::Admin(e)
    }
}
impl From<ostd::Error> for EnaError {
    fn from(e: ostd::Error) -> Self {
        EnaError::Alloc(e)
    }
}

/// One I/O queue pair plus the BAR0 offsets the device handed back.
struct Queue {
    sq: SubmissionRing,
    cq: CompletionRing,
    sq_doorbell: u32,
    cq_head_doorbell: u32,
    cq_unmask: u32,
}

pub struct EnaDevice {
    admin: AdminQueue,
    _msix: CapabilityMsixData,
    mac: EthernetAddress,
    caps: DeviceCapabilities,
    tx: Queue,
    rx: Queue,
    tx_bufs: Vec<Option<TxBuffer>>,
    tx_free_ids: Vec<u16>,
    rx_bufs: Vec<Option<RxBuffer>>,
    rx_doorbell_pending: bool,
    stats: Stats,
}

#[derive(Default, Debug)]
struct Stats {
    rx_packets: u64,
    tx_packets: u64,
    rx_dropped: u64,
}

impl EnaDevice {
    pub(crate) fn init(admin: AdminQueue, mut msix: CapabilityMsixData) -> Result<(), EnaError> {
        let mut admin = admin;

        let attr = admin.get_feature(admin::FEAT_DEVICE_ATTRIBUTES, 0)?;
        let mac = EthernetAddress([
            attr[6] as u8,
            (attr[6] >> 8) as u8,
            (attr[6] >> 16) as u8,
            (attr[6] >> 24) as u8,
            attr[7] as u8,
            (attr[7] >> 8) as u8,
        ]);
        ostd::info!(
            "device {}, impl {:#x} version {:#x}, max mtu {}, features {:#x}",
            mac,
            attr[0],
            attr[1],
            attr[8],
            attr[2]
        );

        // Queue limits: MAX_QUEUES_EXT (v1) on current devices, MAX_QUEUES_NUM on old ones.
        let (max_sq_depth, max_cq_depth) = match admin.get_feature(admin::FEAT_MAX_QUEUES_EXT, 1) {
            Ok(q) => (q[5].min(q[7]), q[6].min(q[8])),
            Err(_) => match admin.get_feature(admin::FEAT_MAX_QUEUES_NUM, 0) {
                Ok(q) => (q[1], q[3]),
                Err(e) => {
                    ostd::warn!(
                        "cannot query queue limits ({:?}); assuming {}",
                        e,
                        IO_QUEUE_DEPTH
                    );
                    (IO_QUEUE_DEPTH as u32, IO_QUEUE_DEPTH as u32)
                }
            },
        };
        ostd::debug!(
            "max sq depth {}, max cq depth {}",
            max_sq_depth,
            max_cq_depth
        );
        if (IO_QUEUE_DEPTH as u32) > max_sq_depth || (IO_QUEUE_DEPTH as u32) > max_cq_depth {
            ostd::warn!(
                "device limits ({}, {}) below ring depth {}",
                max_sq_depth,
                max_cq_depth,
                IO_QUEUE_DEPTH
            );
        }

        // No asynchronous events: the admin interrupt is masked and we never read the AENQ.
        if let Err(e) = admin.set_feature(admin::FEAT_AENQ_CONFIG, &[0, 0]) {
            ostd::warn!("AENQ config not accepted: {:?}", e);
        }
        // Keep frames within one 4 KiB Rx buffer.
        if let Err(e) = admin.set_feature(admin::FEAT_MTU, &[(MTU - 14) as u32]) {
            ostd::warn!("MTU not accepted: {:?}", e);
        }

        // MSI-X: vector 0 is the (masked) admin vector; the queue pair uses vector 1.
        let vector: u16 = if msix.table_size() >= 2 {
            1
        } else {
            return Err(EnaError::NoMsix);
        };

        let rx = Self::create_queue(
            &mut admin,
            admin::SQ_DIRECTION_RX,
            size_of::<RxCdesc>(),
            vector,
        )?;
        let tx = Self::create_queue(
            &mut admin,
            admin::SQ_DIRECTION_TX,
            size_of::<TxCdesc>(),
            vector,
        )?;

        fn on_io_irq(_: &TrapFrame) {
            aster_network::raise_receive_softirq();
            aster_network::raise_send_softirq();
        }
        if let Some(irq) = msix.irq_mut(vector as usize) {
            irq.on_active(on_io_irq);
        }

        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = MTU;
        caps.max_burst_size = None;
        caps.checksum.tcp = Checksum::Both;
        caps.checksum.udp = Checksum::Both;
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.icmpv4 = Checksum::Both;

        let mut device = Self {
            admin,
            _msix: msix,
            mac,
            caps,
            tx,
            rx,
            tx_bufs: (0..IO_QUEUE_DEPTH).map(|_| None).collect(),
            tx_free_ids: (0..IO_QUEUE_DEPTH).rev().collect(),
            rx_bufs: (0..IO_QUEUE_DEPTH).map(|_| None).collect(),
            rx_doorbell_pending: false,
            stats: Stats::default(),
        };

        // Give the device every Rx buffer it can hold (depth - 1 keeps tail != head).
        for req_id in 0..IO_QUEUE_DEPTH - 1 {
            device.add_rx_buffer(req_id)?;
        }
        device.flush_rx_doorbell();
        device.unmask_io_irq();
        // Always visible: the one line that tells an EC2 console reader the NIC is up.
        ostd::early_println!(
            "[kernel] ena: {} ready, {} Rx buffers of {} bytes, {} Tx slots, MSI-X vector {}",
            mac,
            IO_QUEUE_DEPTH - 1,
            RX_BUFFER_LEN,
            IO_QUEUE_DEPTH,
            vector
        );

        aster_network::register_device(DEVICE_NAME.to_string(), Arc::new(SpinLock::new(device)));
        Ok(())
    }

    fn create_queue(
        admin: &mut AdminQueue,
        direction: u8,
        cdesc_size: usize,
        vector: u16,
    ) -> Result<Queue, EnaError> {
        let cq = CompletionRing::new(cdesc_size)?;
        let (cq_idx, cq_head_doorbell, cq_unmask) =
            admin.create_cq(IO_QUEUE_DEPTH, cdesc_size as u16, vector as u32, cq.daddr())?;
        let sq = SubmissionRing::new()?;
        let (_sq_idx, sq_doorbell) =
            admin.create_sq(direction, cq_idx, IO_QUEUE_DEPTH, sq.daddr())?;
        Ok(Queue {
            sq,
            cq,
            sq_doorbell,
            cq_head_doorbell,
            cq_unmask,
        })
    }

    fn write_reg(&self, offset: u32, val: u32) {
        self.admin.bar().write_once(offset as usize, val).unwrap();
    }

    fn add_rx_buffer(&mut self, req_id: u16) -> Result<(), EnaError> {
        let buf = RxBuffer::alloc(RX_POOL.get().unwrap())?;
        let daddr = buf.daddr();
        let desc = RxDesc {
            length: buf.size() as u16,
            ctrl: io::RX_DESC_FIRST
                | io::RX_DESC_LAST
                | io::RX_DESC_COMP_REQ
                | (self.rx.sq.phase & io::RX_DESC_PHASE_MASK),
            req_id,
            buff_addr_lo: daddr as u32,
            buff_addr_hi: (daddr >> 32) as u16,
            ..Default::default()
        };
        debug_assert!(self.rx_bufs[req_id as usize].is_none());
        self.rx_bufs[req_id as usize] = Some(buf);
        self.rx.sq.push(&desc);
        self.rx_doorbell_pending = true;
        Ok(())
    }

    fn flush_rx_doorbell(&mut self) {
        if self.rx_doorbell_pending {
            fence(Ordering::SeqCst);
            self.write_reg(self.rx.sq_doorbell, self.rx.sq.tail as u32);
            self.rx_doorbell_pending = false;
        }
    }

    fn report_cq_heads(&mut self) {
        for q in [&mut self.rx, &mut self.tx] {
            if q.cq_head_doorbell != 0 && q.cq.head != q.cq.last_head_reported {
                q.cq.last_head_reported = q.cq.head;
                let (off, head) = (q.cq_head_doorbell, q.cq.head as u32);
                self.admin.bar().write_once(off as usize, head).unwrap();
            }
        }
    }

    fn unmask_io_irq(&self) {
        self.write_reg(self.rx.cq_unmask, io::INTR_UNMASK);
    }

    fn do_receive(&mut self) -> Result<RxPacket<LinkLayer>, NetError> {
        loop {
            let Some(cdesc) = self.rx.cq.pop::<RxCdesc>(3, 0) else {
                return Err(NetError::NotReady);
            };
            let req_id = cdesc.req_id;
            if req_id >= IO_QUEUE_DEPTH {
                ostd::error!("rx completion with bad req_id {}", req_id);
                continue;
            }
            let Some(buf) = self.rx_bufs[req_id as usize].take() else {
                ostd::error!("rx completion for empty slot {}", req_id);
                continue;
            };
            self.rx.sq.next_to_comp = self.rx.sq.next_to_comp.wrapping_add(1);
            // Hand the slot straight back to the device.
            if let Err(e) = self.add_rx_buffer(req_id) {
                ostd::error!("cannot refill rx slot {}: {:?}", req_id, e);
            }

            let whole =
                cdesc.status & io::RX_CDESC_FIRST != 0 && cdesc.status & io::RX_CDESC_LAST != 0;
            if !whole {
                // Frames larger than one buffer are not expected with MTU 1500; drop the pieces.
                self.stats.rx_dropped += 1;
                ostd::debug!(
                    "dropping multi-descriptor rx frame (status {:#x})",
                    cdesc.status
                );
                continue;
            }
            let offset = cdesc.offset as usize;
            let len = cdesc.length as usize;
            if offset + len > buf.size() || offset > 12 {
                self.stats.rx_dropped += 1;
                ostd::warn!("rx frame outside its buffer: offset {} len {}", offset, len);
                continue;
            }
            self.stats.rx_packets += 1;
            ostd::debug!(
                "rx {} bytes (req {}, status {:#x})",
                len,
                req_id,
                cdesc.status
            );
            let packet = buf.finish_dma(offset + len);
            return Ok(packet.peel(offset));
        }
    }

    fn do_send(&mut self, packet: TxPacket<LinkLayer>) -> Result<(), NetError> {
        if !self.can_send() {
            return Err(NetError::Busy);
        }
        let len = packet.len();
        // No device header: repack the link-layer frame as the device layer.
        let packet = packet.pack(0);
        let tx_buffer = packet.map_dma(false).map_err(|_| NetError::NoMemory)?;
        let daddr = tx_buffer.daddr();
        let req_id = self.tx_free_ids.pop().unwrap();
        let desc = TxDesc {
            len_ctrl: (len as u32 & io::TX_DESC_LENGTH_MASK)
                | (((req_id as u32) >> 10) << io::TX_DESC_REQ_ID_HI_SHIFT)
                | ((self.tx.sq.phase as u32) << io::TX_DESC_PHASE_SHIFT)
                | io::TX_DESC_FIRST
                | io::TX_DESC_LAST
                | io::TX_DESC_COMP_REQ,
            meta_ctrl: ((req_id as u32) & 0x3ff) << io::TX_DESC_REQ_ID_LO_SHIFT,
            buff_addr_lo: daddr as u32,
            buff_addr_hi_hdr_sz: ((daddr >> 32) as u32) & io::TX_DESC_ADDR_HI_MASK,
        };
        self.tx_bufs[req_id as usize] = Some(tx_buffer);
        self.tx.sq.push(&desc);
        fence(Ordering::SeqCst);
        self.write_reg(self.tx.sq_doorbell, self.tx.sq.tail as u32);
        self.stats.tx_packets += 1;
        ostd::debug!("tx {} bytes (req {})", len, req_id);
        Ok(())
    }
}

impl AnyNetworkDevice for EnaDevice {
    fn mac_addr(&self) -> EthernetAddress {
        self.mac
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.caps.clone()
    }

    fn can_receive(&self) -> bool {
        self.rx.cq.has_entry(3, 0)
    }

    fn can_send(&self) -> bool {
        self.tx.sq.free_entries() >= 1 && !self.tx_free_ids.is_empty()
    }

    fn receive(&mut self) -> Result<RxPacket<LinkLayer>, NetError> {
        self.do_receive()
    }

    fn send(&mut self, packet: TxPacket<LinkLayer>) -> Result<(), NetError> {
        self.do_send(packet)
    }

    fn alloc_tx_buffer(payload_len: usize) -> Result<FreshTxPacket, ostd::Error> {
        if payload_len > const { TX_BUFFER_LEN - ApplicationLayer::HEAD_ROOM_SIZE } {
            return Err(ostd::Error::InvalidArgs);
        }
        FreshTxPacket::alloc_from_pool(TX_POOL.get().unwrap())
    }

    fn free_processed_tx_buffers(&mut self) {
        while let Some(cdesc) = self.tx.cq.pop::<TxCdesc>(3, 0) {
            let req_id = cdesc.req_id;
            if req_id >= IO_QUEUE_DEPTH || self.tx_bufs[req_id as usize].is_none() {
                ostd::error!("tx completion with bad req_id {}", req_id);
                continue;
            }
            self.tx_bufs[req_id as usize] = None;
            self.tx_free_ids.push(req_id);
            self.tx.sq.next_to_comp = self.tx.sq.next_to_comp.wrapping_add(1);
        }
    }

    fn notify_poll_end(&mut self) {
        self.flush_rx_doorbell();
        self.report_cq_heads();
        self.unmask_io_irq();
    }
}

impl Debug for EnaDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EnaDevice")
            .field("mac", &self.mac)
            .field("tx_tail", &self.tx.sq.tail)
            .field("tx_next_to_comp", &self.tx.sq.next_to_comp)
            .field("rx_tail", &self.rx.sq.tail)
            .field("rx_cq_head", &self.rx.cq.head)
            .field("stats", &self.stats)
            .finish()
    }
}
