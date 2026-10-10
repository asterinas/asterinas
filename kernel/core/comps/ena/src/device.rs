// SPDX-License-Identifier: MPL-2.0

//! The ENA network device: several Tx/Rx queue pairs with RSS, Tx checksum
//! offload, MSI-X interrupts plus a timer-tick poll, an AENQ keep-alive
//! watchdog and a device reset path.

use alloc::{string::ToString, sync::Arc, vec::Vec};
use core::{
    fmt::Debug,
    sync::atomic::{AtomicU32, AtomicU64, Ordering, fence},
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
    admin::{self, AdminError, AdminQueue, AenqEvent},
    io::{
        self, CompletionRing, IO_QUEUE_DEPTH, RxCdesc, RxDesc, SubmissionRing, TxCdesc, TxDesc,
        TxMetaDesc,
    },
};

pub const DEVICE_NAME: &str = "ENA";

const RX_BUFFER_LEN: usize = 4096;
pub(crate) const TX_BUFFER_LEN: usize = 4096;
/// Link-layer MTU: 1500 bytes of IP plus the Ethernet header.
const MTU: usize = 1514;
/// Upper bound on queue pairs; the device, the MSI-X table and the vCPU count
/// cap it further.
const MAX_QUEUES: usize = 8;
/// Keep-alive: the device sends one every second; Linux declares it dead after 6 s.
const KEEP_ALIVE_TIMEOUT_MS: u64 = 6_000;
/// RSS indirection table: 128 entries, like Linux.
const RSS_TABLE_LOG_SIZE: u16 = 7;
/// Toeplitz key (Microsoft's reference key; any 40 bytes work).
const RSS_KEY: [u8; 40] = [
    0x6d, 0x5a, 0x56, 0xda, 0x25, 0x5b, 0x0e, 0xc2, 0x41, 0x67, 0x25, 0x3d, 0x43, 0xa3, 0x8f, 0xb0,
    0xd0, 0xca, 0x2b, 0xcb, 0xae, 0x7b, 0x30, 0xb4, 0x77, 0xcb, 0x2d, 0xa3, 0x80, 0x30, 0xf2, 0x0c,
    0x6a, 0x42, 0xb7, 0x3b, 0xbe, 0xac, 0x01, 0xfa,
];

static RX_POOL: Once<Arc<DmaPool<FromDevice>>> = Once::new();
static TX_POOL: Once<Arc<DmaPool<ToDevice>>> = Once::new();

pub(crate) fn init_pools() {
    RX_POOL.call_once(|| DmaPool::new(RX_BUFFER_LEN, 64, 1024, false));
    TX_POOL.call_once(|| DmaPool::new(TX_BUFFER_LEN, 32, 512, false));
}

/// Milliseconds since boot as seen by the timer tick (`TIMER_FREQ` = 1000).
pub(crate) static TICK_MS: AtomicU64 = AtomicU64::new(0);
/// Set by the admin MSI-X vector: the AENQ has something to read.
pub(crate) static AENQ_PENDING: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
/// Set by the timer tick: run `tick()` on the next softirq poll.
pub(crate) static TICK_DUE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

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

/// One Tx or Rx queue: a submission ring, its completion ring and the BAR0
/// offsets the device handed back.
struct Queue {
    sq: SubmissionRing,
    cq: CompletionRing,
    sq_doorbell: u32,
    cq_head_doorbell: u32,
    cq_unmask: u32,
    cq_idx: u16,
}

struct TxQueue {
    q: Queue,
    bufs: Vec<Option<TxBuffer>>,
    free_ids: Vec<u16>,
    /// Descriptors consumed per request (1, or 2 when a meta descriptor preceded it).
    descs: Vec<u8>,
    /// Whether the (cached) meta descriptor has been sent on this queue since
    /// the last reset; the device remembers it until a new one arrives.
    meta_sent: bool,
}

struct RxQueue {
    q: Queue,
    bufs: Vec<Option<RxBuffer>>,
    doorbell_pending: bool,
}

/// What the device offloads. `tx_l4_partial` means we leave the pseudo-header
/// checksum in place and the device finishes it; `rx_l4` means the device
/// verified it and told us in the completion.
#[derive(Clone, Copy, Debug, Default)]
#[expect(dead_code)] // reported in `{:?}` only
struct Offloads {
    tx_l3: bool,
    tx_l4_partial: bool,
    tx_l4_full: bool,
    rx_l3: bool,
    rx_l4: bool,
}

pub struct EnaDevice {
    admin: AdminQueue,
    msix: CapabilityMsixData,
    mac: EthernetAddress,
    caps: DeviceCapabilities,
    offloads: Offloads,
    tx: Vec<TxQueue>,
    rx: Vec<RxQueue>,
    /// Last Tx queue used (diagnostics).
    next_tx: usize,
    /// Round-robin position for draining Rx completions across queues.
    next_rx: usize,
    stats: Stats,
    /// `TICK_MS` when the last keep-alive arrived.
    last_keep_alive_ms: u64,
    keep_alive_enabled: bool,
    link_up: bool,
    resets: u32,
    /// Set by `ena.test_reset=N` on the command line: force a reset N seconds after init.
    test_reset_at_ms: Option<u64>,
    /// Why a reset is pending; performed from softirq context, not from the tick.
    reset_pending: Option<&'static str>,
    /// Ticks left during which `tick()` prints queue state (after a reset).
    debug_ticks: u32,
    /// `TICK_MS` of the last reset, for the post-reset liveness check.
    last_reset_ms: Option<u64>,
    /// Host tx count at the last reset.
    tx_at_reset: u64,
}

#[derive(Default, Debug)]
struct Stats {
    rx_packets: u64,
    tx_packets: u64,
    rx_dropped: u64,
    rx_csum_bad: u64,
    rx_drops_dev: u64,
    tx_drops_dev: u64,
}

/// Number of queue pairs to create: `ena.queues=N`, default (`auto`) one per
/// vCPU up to `MAX_QUEUES`. Tx is steered by flow hash so a connection stays
/// on one queue; RSS does the same for Rx.
fn wanted_queues() -> usize {
    let cpus = ostd::cpu::num_cpus() as usize;
    match crate::QUEUES_PARAM.get().map(|v| v.as_str()) {
        None | Some("auto") => cpus,
        Some(v) => v.parse::<usize>().unwrap_or(cpus),
    }
    .clamp(1, MAX_QUEUES)
}

impl EnaDevice {
    pub(crate) fn init(
        mut admin: AdminQueue,
        mut msix: CapabilityMsixData,
    ) -> Result<(), EnaError> {
        let attr = admin.get_feature(admin::FEAT_DEVICE_ATTRIBUTES, 0)?;
        let mac = EthernetAddress([
            attr[6] as u8,
            (attr[6] >> 8) as u8,
            (attr[6] >> 16) as u8,
            (attr[6] >> 24) as u8,
            attr[7] as u8,
            (attr[7] >> 8) as u8,
        ]);
        let supported_features = attr[2];
        ostd::info!(
            "device {}, impl {:#x} version {:#x}, max mtu {}, features {:#x}",
            mac,
            attr[0],
            attr[1],
            attr[8],
            supported_features
        );

        // Queue limits: MAX_QUEUES_EXT (v1) on current devices, MAX_QUEUES_NUM on old ones.
        let (max_sq_num, max_sq_depth, max_cq_depth) =
            match admin.get_feature(admin::FEAT_MAX_QUEUES_EXT, 1) {
                Ok(q) => (
                    q[1].min(q[2]).min(q[3]).min(q[4]),
                    q[5].min(q[7]),
                    q[6].min(q[8]),
                ),
                Err(_) => match admin.get_feature(admin::FEAT_MAX_QUEUES_NUM, 0) {
                    Ok(q) => (q[0], q[1], q[3]),
                    Err(e) => {
                        ostd::warn!(
                            "cannot query queue limits ({:?}); assuming 1 x {}",
                            e,
                            IO_QUEUE_DEPTH
                        );
                        (1, IO_QUEUE_DEPTH as u32, IO_QUEUE_DEPTH as u32)
                    }
                },
            };
        if (IO_QUEUE_DEPTH as u32) > max_sq_depth || (IO_QUEUE_DEPTH as u32) > max_cq_depth {
            ostd::warn!(
                "device limits ({}, {}) below ring depth {}",
                max_sq_depth,
                max_cq_depth,
                IO_QUEUE_DEPTH
            );
        }

        // MSI-X: vector 0 is the (masked) admin vector; queue pair i uses vector i+1.
        let vectors_for_io = msix.table_size().saturating_sub(1) as usize;
        if vectors_for_io == 0 {
            return Err(EnaError::NoMsix);
        }
        let num_queues = wanted_queues()
            .min(max_sq_num as usize)
            .min(vectors_for_io)
            .max(1);

        let offloads = if crate::OFFLOAD_PARAM.get().is_some_and(|v| v == "0") {
            ostd::info!("checksum offload disabled by ena.offload=0");
            Offloads::default()
        } else {
            Self::query_offloads(&mut admin)
        };
        // Keep frames within one 4 KiB Rx buffer.
        if let Err(e) = admin.set_feature(admin::FEAT_MTU, &[(MTU - 14) as u32]) {
            ostd::warn!("MTU not accepted: {:?}", e);
        }

        fn on_io_irq(_: &TrapFrame) {
            aster_network::raise_receive_softirq();
            aster_network::raise_send_softirq();
        }
        for v in 1..=num_queues {
            if let Some(irq) = msix.irq_mut(v) {
                irq.on_active(on_io_irq);
            }
        }
        // Vector 0: admin/AENQ. Completions are polled, but an AENQ event
        // (keep-alive, link change, fatal error) should be looked at soon.
        fn on_admin_irq(_: &TrapFrame) {
            AENQ_PENDING.store(true, Ordering::Release);
        }
        if let Some(irq) = msix.irq_mut(0) {
            irq.on_active(on_admin_irq);
        }

        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = MTU;
        caps.max_burst_size = None;
        // smoltcp still computes IPv4/ICMP checksums; TCP/UDP are offloaded on
        // Tx when the device supports partial L4 checksum (we hand it the
        // pseudo-header sum) and verified by the device on Rx.
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.icmpv4 = Checksum::Both;
        let l4 = match (offloads.tx_l4_partial, offloads.rx_l4) {
            (true, true) => Checksum::None,
            (true, false) => Checksum::Rx,
            (false, true) => Checksum::Tx,
            (false, false) => Checksum::Both,
        };
        caps.checksum.tcp = l4;
        caps.checksum.udp = l4;

        let test_reset_at_ms = crate::TEST_RESET_PARAM
            .get()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|secs| TICK_MS.load(Ordering::Relaxed) + secs * 1000);

        let mut device = Self {
            admin,
            msix,
            mac,
            caps,
            offloads,
            tx: Vec::new(),
            rx: Vec::new(),
            next_tx: 0,
            next_rx: 0,
            stats: Stats::default(),
            last_keep_alive_ms: TICK_MS.load(Ordering::Relaxed),
            keep_alive_enabled: false,
            link_up: true,
            resets: 0,
            test_reset_at_ms,
            reset_pending: None,
            debug_ticks: 0,
            last_reset_ms: None,
            tx_at_reset: 0,
        };
        device.setup_queues(num_queues)?;
        device.setup_aenq();

        ostd::early_println!(
            "[kernel] ena: {} ready, {} queue pair{} of {} Rx buffers x {} bytes, tx csum offload {}, rss {}",
            mac,
            device.rx.len(),
            if device.rx.len() == 1 { "" } else { "s" },
            IO_QUEUE_DEPTH - 1,
            RX_BUFFER_LEN,
            if device.offloads.tx_l4_partial {
                "on"
            } else {
                "off"
            },
            if device.rx.len() > 1 { "on" } else { "off" }
        );

        aster_network::register_device(DEVICE_NAME.to_string(), Arc::new(SpinLock::new(device)));
        Ok(())
    }

    fn query_offloads(admin: &mut AdminQueue) -> Offloads {
        match admin.get_feature(admin::FEAT_STATELESS_OFFLOAD_CONFIG, 0) {
            Ok(o) => {
                let tx = o[0];
                let rx = o[2]; // rx_enabled
                let off = Offloads {
                    tx_l3: tx & 1 != 0,
                    tx_l4_partial: tx & (1 << 1) != 0,
                    tx_l4_full: tx & (1 << 2) != 0,
                    rx_l3: rx & 1 != 0,
                    rx_l4: rx & (1 << 1) != 0,
                };
                ostd::info!(
                    "stateless offload: tx {:#x} rx_supported {:#x} rx_enabled {:#x} -> {:?}",
                    o[0],
                    o[1],
                    o[2],
                    off
                );
                off
            }
            Err(e) => {
                ostd::warn!(
                    "STATELESS_OFFLOAD_CONFIG not available ({:?}); checksums in software",
                    e
                );
                Offloads::default()
            }
        }
    }

    /// Creates `n` queue pairs, fills the Rx rings and configures RSS.
    fn setup_queues(&mut self, n: usize) -> Result<(), EnaError> {
        self.tx.clear();
        self.rx.clear();
        for i in 0..n {
            let vector = (i + 1) as u16;
            let rxq = Self::create_queue(
                &mut self.admin,
                admin::SQ_DIRECTION_RX,
                size_of::<RxCdesc>(),
                vector,
            )?;
            let txq = Self::create_queue(
                &mut self.admin,
                admin::SQ_DIRECTION_TX,
                size_of::<TxCdesc>(),
                vector,
            )?;
            self.rx.push(RxQueue {
                q: rxq,
                bufs: (0..IO_QUEUE_DEPTH).map(|_| None).collect(),
                doorbell_pending: false,
            });
            self.tx.push(TxQueue {
                q: txq,
                bufs: (0..IO_QUEUE_DEPTH).map(|_| None).collect(),
                free_ids: (0..IO_QUEUE_DEPTH).rev().collect(),
                descs: alloc::vec![0; IO_QUEUE_DEPTH as usize],
                meta_sent: false,
            });
        }
        for qi in 0..n {
            // depth - 1 keeps tail != head
            for req_id in 0..IO_QUEUE_DEPTH - 1 {
                self.add_rx_buffer(qi, req_id)?;
            }
            self.flush_rx_doorbell(qi);
        }
        if n > 1
            && let Err(e) = self.setup_rss(n)
        {
            ostd::warn!("RSS setup failed ({:?}); traffic lands on queue 0 only", e);
        }
        for qi in 0..n {
            self.unmask_io_irq(qi);
        }
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
            cq_idx,
        })
    }

    /// RSS: Toeplitz over the IPv4 5-tuple, indirection table spread round
    /// robin over the Rx completion queues.
    fn setup_rss(&mut self, n: usize) -> Result<(), AdminError> {
        const TOEPLITZ: u32 = 1 << 1;
        // Hash function + key: inline func/init_val, key in the control buffer
        // as `ena_admin_feature_rss_flow_hash_control` (key_parts, reserved, key[10]).
        let mut ctrl = Vec::with_capacity(48);
        ctrl.extend_from_slice(&(10u32).to_le_bytes());
        ctrl.extend_from_slice(&0u32.to_le_bytes());
        ctrl.extend_from_slice(&RSS_KEY);
        let func = self.admin.get_feature(admin::FEAT_RSS_HASH_FUNCTION, 0)?;
        let selected = if func[0] & TOEPLITZ != 0 {
            TOEPLITZ
        } else {
            1 << 2 /* CRC32 */
        };
        self.admin.set_feature_indirect(
            admin::FEAT_RSS_HASH_FUNCTION,
            &[func[0], selected, 0x5a5a_5a5a],
            &ctrl,
        )?;

        // Hash input: for each protocol (TCP4=0, UDP4=1, IP4=4, ...) the fields
        // to hash. The table is 10 x (supported, enabled) u16 pairs in the
        // control buffer; set L3 SA/DA + L4 SP/DP for TCP4/UDP4, L3 for IP4.
        const L3: u16 = (1 << 2) | (1 << 3);
        const L4: u16 = (1 << 4) | (1 << 5);
        let mut input = alloc::vec![0u8; 10 * 4];
        for proto in 0..10u16 {
            let fields = match proto {
                0 | 1 => L3 | L4, // TCP4, UDP4
                4 | 6 => L3,      // IP4, IP4_FRAG
                _ => 0,
            };
            input[(proto * 4) as usize..(proto * 4 + 2) as usize]
                .copy_from_slice(&fields.to_le_bytes());
            input[(proto * 4 + 2) as usize..(proto * 4 + 4) as usize]
                .copy_from_slice(&fields.to_le_bytes());
        }
        if let Err(e) = self
            .admin
            .set_feature_indirect(admin::FEAT_RSS_HASH_INPUT, &[], &input)
        {
            ostd::debug!(
                "RSS hash input not accepted ({:?}); device default applies",
                e
            );
        }

        // Indirection table: 2^7 entries of (cq_idx u16, reserved u16).
        let size = 1usize << RSS_TABLE_LOG_SIZE;
        let mut table = Vec::with_capacity(size * 4);
        for i in 0..size {
            table.extend_from_slice(&self.rx[i % n].q.cq_idx.to_le_bytes());
            table.extend_from_slice(&0u16.to_le_bytes());
        }
        // inline: size (u16 min, u16 max) packed in word0 as Linux's struct: min_size, max_size, size, reserved, inline_index
        let word0 = (RSS_TABLE_LOG_SIZE as u32) << 16; // min_size=0, max_size=0 (ignored on set)
        let word1 = RSS_TABLE_LOG_SIZE as u32; // size (u16) | reserved
        let word2 = 0xffff_ffff; // inline_index: none
        self.admin.set_feature_indirect(
            admin::FEAT_RSS_INDIRECTION_TABLE,
            &[word0, word1, word2],
            &table,
        )?;
        ostd::info!(
            "RSS: {} entries over {} queues, func {:#x}",
            size,
            n,
            selected
        );
        Ok(())
    }

    fn setup_aenq(&mut self) {
        let want = admin::AENQ_GROUP_LINK_CHANGE
            | admin::AENQ_GROUP_FATAL_ERROR
            | admin::AENQ_GROUP_WARNING
            | admin::AENQ_GROUP_NOTIFICATION
            | admin::AENQ_GROUP_KEEP_ALIVE;
        match self.admin.enable_aenq_groups(want) {
            Ok(enabled) => {
                self.keep_alive_enabled = enabled & admin::AENQ_GROUP_KEEP_ALIVE != 0;
                self.last_keep_alive_ms = TICK_MS.load(Ordering::Relaxed);
                ostd::info!(
                    "AENQ groups enabled {:#x} (keep-alive {})",
                    enabled,
                    self.keep_alive_enabled
                );
            }
            Err(e) => ostd::warn!("AENQ config not accepted: {:?}", e),
        }
    }

    fn write_reg(&self, offset: u32, val: u32) {
        self.admin.bar().write_once(offset as usize, val).unwrap();
    }

    fn add_rx_buffer(&mut self, qi: usize, req_id: u16) -> Result<(), EnaError> {
        let buf = RxBuffer::alloc(RX_POOL.get().unwrap())?;
        let daddr = buf.daddr();
        let rxq = &mut self.rx[qi];
        let desc = RxDesc {
            length: buf.size() as u16,
            ctrl: io::RX_DESC_FIRST
                | io::RX_DESC_LAST
                | io::RX_DESC_COMP_REQ
                | (rxq.q.sq.phase & io::RX_DESC_PHASE_MASK),
            req_id,
            buff_addr_lo: daddr as u32,
            buff_addr_hi: (daddr >> 32) as u16,
            ..Default::default()
        };
        debug_assert!(rxq.bufs[req_id as usize].is_none());
        rxq.bufs[req_id as usize] = Some(buf);
        rxq.q.sq.push(&desc);
        rxq.doorbell_pending = true;
        Ok(())
    }

    fn flush_rx_doorbell(&mut self, qi: usize) {
        if self.rx[qi].doorbell_pending {
            fence(Ordering::SeqCst);
            let (off, tail) = (self.rx[qi].q.sq_doorbell, self.rx[qi].q.sq.tail as u32);
            self.write_reg(off, tail);
            self.rx[qi].doorbell_pending = false;
        }
    }

    fn report_cq_heads(&mut self) {
        let mut writes = Vec::new();
        for q in self
            .rx
            .iter_mut()
            .map(|r| &mut r.q)
            .chain(self.tx.iter_mut().map(|t| &mut t.q))
        {
            if q.cq_head_doorbell != 0 && q.cq.head != q.cq.last_head_reported {
                q.cq.last_head_reported = q.cq.head;
                writes.push((q.cq_head_doorbell, q.cq.head as u32));
            }
        }
        for (off, head) in writes {
            self.write_reg(off, head);
        }
    }

    fn unmask_io_irq(&self, qi: usize) {
        self.write_reg(self.rx[qi].q.cq_unmask, io::INTR_UNMASK);
    }

    fn do_receive(&mut self) -> Result<RxPacket<LinkLayer>, NetError> {
        // Health work runs here, in softirq context with the device lock
        // held by the poller, never in the timer interrupt: a readless MMIO
        // read or an admin command can take milliseconds (the device
        // answers by DMA), which is fine in a softirq and fatal in an IRQ.
        if TICK_DUE.swap(false, Ordering::AcqRel) {
            self.tick();
        }
        if let Some(why) = self.reset_pending.take() {
            self.reset(why);
            return Err(NetError::NotReady);
        }
        let n = self.rx.len();
        // Round robin over queues so one busy flow cannot starve the others.
        for step in 0..n {
            let qi = (self.next_rx + step) % n;
            while let Some(cdesc) = self.rx[qi].q.cq.pop::<RxCdesc>(3, 0) {
                let req_id = cdesc.req_id;
                if req_id >= IO_QUEUE_DEPTH {
                    ostd::error!("rx completion with bad req_id {} on queue {}", req_id, qi);
                    continue;
                }
                let Some(buf) = self.rx[qi].bufs[req_id as usize].take() else {
                    ostd::error!("rx completion for empty slot {} on queue {}", req_id, qi);
                    continue;
                };
                self.rx[qi].q.sq.next_to_comp = self.rx[qi].q.sq.next_to_comp.wrapping_add(1);
                if let Err(e) = self.add_rx_buffer(qi, req_id) {
                    ostd::error!("cannot refill rx slot {}: {:?}", req_id, e);
                }

                let whole =
                    cdesc.status & io::RX_CDESC_FIRST != 0 && cdesc.status & io::RX_CDESC_LAST != 0;
                if !whole {
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
                // The device verified L3/L4 checksums; drop frames it flagged bad
                // (smoltcp is told not to re-verify L4 when rx_l4 is on).
                if self.offloads.rx_l4
                    && cdesc.status & io::RX_CDESC_L4_CSUM_CHECKED != 0
                    && cdesc.status & io::RX_CDESC_L4_CSUM_ERR != 0
                {
                    self.stats.rx_csum_bad += 1;
                    continue;
                }
                self.stats.rx_packets += 1;
                self.next_rx = (qi + 1) % n;
                let packet = buf.finish_dma(offset + len);
                return Ok(packet.peel(offset));
            }
        }
        Err(NetError::NotReady)
    }

    /// Prepares an Ethernet/IPv4/TCP|UDP frame for partial L4 checksum
    /// offload: writes the pseudo-header checksum into the L4 checksum field
    /// (the device adds the payload sum, like Linux's `CHECKSUM_PARTIAL`).
    /// Returns `(l3_offset, l3_len, l4_proto)` or `None` for other frames.
    fn prepare_l4_offload(packet: &mut TxPacket<LinkLayer>) -> Option<(u8, u8, u8)> {
        let mut hdr = [0u8; 34];
        let mut reader = packet.reader();
        let total = reader.remain();
        if total < hdr.len() {
            return None;
        }
        reader.read(&mut ostd::mm::VmWriter::from(&mut hdr[..]));
        if hdr[12] != 0x08 || hdr[13] != 0x00 {
            return None; // not IPv4
        }
        let ihl = ((hdr[14] & 0x0f) * 4) as usize;
        if hdr[14] >> 4 != 4 || ihl < 20 || hdr[20] & 0x3f != 0 || hdr[21] != 0 {
            return None; // bad header or a fragment
        }
        let proto = hdr[23];
        let csum_off = match proto {
            6 => 16, // TCP
            17 => 6, // UDP
            _ => return None,
        };
        let ip_total_len = u16::from_be_bytes([hdr[16], hdr[17]]) as usize;
        if ip_total_len < ihl || 14 + ip_total_len > total {
            return None;
        }
        let l4_len = (ip_total_len - ihl) as u32;
        let l4_off = 14 + ihl;
        if l4_off + csum_off + 2 > total {
            return None;
        }
        // Pseudo header: src, dst, zero, proto, L4 length.
        let mut sum: u32 = 0;
        for i in (26..34).step_by(2) {
            sum += u16::from_be_bytes([hdr[i], hdr[i + 1]]) as u32;
        }
        sum += proto as u32;
        sum += l4_len;
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        let pseudo = (sum as u16).to_be_bytes();
        packet
            .data_writer(l4_off + csum_off, 2)
            .write(&mut ostd::mm::VmReader::from(&pseudo[..]));
        Some((14, ihl as u8, proto))
    }

    /// Picks the Tx queue for a frame: the same flow (IPv4 5-tuple) always goes
    /// to the same queue so segments of one TCP connection stay in order; the
    /// device would otherwise interleave two queues and the receiver sees
    /// reordering, which halved single-flow throughput with two pairs.
    fn tx_queue_for(&self, packet: &TxPacket<LinkLayer>) -> usize {
        let n = self.tx.len();
        if n == 1 {
            return 0;
        }
        let mut hdr = [0u8; 38];
        let mut reader = packet.reader();
        if reader.remain() < hdr.len() {
            return 0;
        }
        reader.read(&mut ostd::mm::VmWriter::from(&mut hdr[..]));
        if hdr[12] != 0x08 || hdr[13] != 0x00 || hdr[14] >> 4 != 4 {
            return 0; // non-IPv4 (ARP etc.) on queue 0
        }
        let ihl = ((hdr[14] & 0x0f) * 4) as usize;
        let mut h: u32 = 0x811c_9dc5;
        let mut mix = |b: u8| {
            h ^= b as u32;
            h = h.wrapping_mul(0x0100_0193);
        };
        for b in &hdr[26..34] {
            mix(*b); // src + dst
        }
        mix(hdr[23]); // proto
        let l4 = 14 + ihl;
        if (hdr[23] == 6 || hdr[23] == 17) && l4 + 4 <= hdr.len() {
            for b in &hdr[l4..l4 + 4] {
                mix(*b); // ports
            }
        }
        (h as usize) % n
    }

    fn do_send(&mut self, mut packet: TxPacket<LinkLayer>) -> Result<(), NetError> {
        let n = self.tx.len();
        let preferred = self.tx_queue_for(&packet);
        let mut qi = None;
        for step in 0..n {
            let i = (preferred + step) % n;
            if self.tx[i].q.sq.free_entries() >= 2 && !self.tx[i].free_ids.is_empty() {
                qi = Some(i);
                break;
            }
        }
        let Some(qi) = qi else {
            static ONCE: AtomicU32 = AtomicU32::new(0);
            if ONCE.fetch_add(1, Ordering::Relaxed) < 3 {
                let q: Vec<(u16, u16, u16, usize)> = self
                    .tx
                    .iter()
                    .map(|t| {
                        (
                            t.q.sq.tail,
                            t.q.sq.next_to_comp,
                            t.q.sq.free_entries(),
                            t.free_ids.len(),
                        )
                    })
                    .collect();
                ostd::early_println!(
                    "[kernel] ena: tx busy: {} queues {:?}, reset_pending {:?}",
                    n,
                    q,
                    self.reset_pending
                );
            }
            return Err(NetError::Busy);
        };
        self.next_tx = qi;

        let len = packet.len();
        // Checksum offload: TCP/UDP over IPv4 when the device does partial L4
        // (smoltcp left the pseudo-header checksum in the L4 header).
        let csum = if self.offloads.tx_l4_partial {
            Self::prepare_l4_offload(&mut packet)
        } else {
            None
        };
        let (l3_proto, l4_proto, l3_off, l3_len, l4_csum) = match csum {
            Some((off, ihl, 6)) => (io::L3_PROTO_IPV4, io::L4_PROTO_TCP, off, ihl, true),
            Some((off, ihl, 17)) => (io::L3_PROTO_IPV4, io::L4_PROTO_UDP, off, ihl, true),
            _ => (0, 0, 0, 0, false),
        };

        // No device header: repack the link-layer frame as the device layer.
        let packet = packet.pack(0);
        let tx_buffer = packet.map_dma(false).map_err(|_| NetError::NoMemory)?;
        let daddr = tx_buffer.daddr();
        let txq = &mut self.tx[qi];
        let req_id = txq.free_ids.pop().unwrap();

        // Meta descriptor carries L3/L4 header geometry; the device caches it
        // (META_STORE), so send it only when the geometry changes. We always
        // use the same geometry (Ethernet + IPv4), so once per queue.
        let mut first = true;
        if l4_csum && !txq.meta_sent {
            let meta = TxMetaDesc {
                len_ctrl: io::TX_META_DESC_META_DESC
                    | io::TX_META_DESC_EXT_VALID
                    | io::TX_META_DESC_ETH_META_TYPE
                    | io::TX_META_DESC_META_STORE
                    | io::TX_META_DESC_FIRST
                    | ((txq.q.sq.phase as u32) << io::TX_DESC_PHASE_SHIFT),
                word1: 0,
                word2: (l3_len as u32 & 0xff) | ((l3_off as u32) << 8),
                reserved: 0,
            };
            txq.q.sq.push(&meta);
            txq.meta_sent = true;
            first = false;
        }
        let mut meta_ctrl = ((req_id as u32) & 0x3ff) << io::TX_DESC_REQ_ID_LO_SHIFT;
        if l4_csum {
            meta_ctrl |= (l3_proto as u32) & 0xf;
            meta_ctrl |= ((l4_proto as u32) << io::TX_DESC_L4_PROTO_IDX_SHIFT)
                & io::TX_DESC_L4_PROTO_IDX_MASK;
            meta_ctrl |= io::TX_DESC_L4_CSUM_EN | io::TX_DESC_L4_CSUM_PARTIAL;
            if self.offloads.tx_l3 {
                meta_ctrl |= io::TX_DESC_L3_CSUM_EN;
            }
        }
        let desc = TxDesc {
            len_ctrl: (len as u32 & io::TX_DESC_LENGTH_MASK)
                | (((req_id as u32) >> 10) << io::TX_DESC_REQ_ID_HI_SHIFT)
                | ((txq.q.sq.phase as u32) << io::TX_DESC_PHASE_SHIFT)
                | if first { io::TX_DESC_FIRST } else { 0 }
                | io::TX_DESC_LAST
                | io::TX_DESC_COMP_REQ,
            meta_ctrl,
            buff_addr_lo: daddr as u32,
            buff_addr_hi_hdr_sz: ((daddr >> 32) as u32) & io::TX_DESC_ADDR_HI_MASK,
        };
        txq.bufs[req_id as usize] = Some(tx_buffer);
        txq.descs[req_id as usize] = if first { 1 } else { 2 };
        txq.q.sq.push(&desc);
        fence(Ordering::SeqCst);
        let (off, tail) = (txq.q.sq_doorbell, txq.q.sq.tail as u32);
        self.write_reg(off, tail);
        self.stats.tx_packets += 1;
        ostd::debug!(
            "tx {} bytes (queue {}, req {}, csum {})",
            len,
            qi,
            req_id,
            l4_csum
        );
        Ok(())
    }

    // ---------------------------------------------------------------- health

    /// Health work, every 100 ms (scheduled by the timer, run from the
    /// softirq): drains the AENQ, watches the keep-alive, checks the data
    /// path after a reset, and triggers a reset when the device is gone.
    fn tick(&mut self) {
        AENQ_PENDING.store(false, Ordering::Relaxed);
        let now = TICK_MS.load(Ordering::Relaxed);
        // Post-reset liveness: the admin path comes back after a reset but on
        // Nitro the data path has not (device Rx/Tx counters stay at zero, see
        // the patch notes). If the stack has sent frames since the reset and
        // the device counted none within 10 s, the NIC is lost; reboot the
        // machine, which restores service (EC2 then re-runs the same image).
        if let Some(at) = self.last_reset_ms
            && now.saturating_sub(at) > 10_000
        {
            self.last_reset_ms = None;
            let sent_since = self.stats.tx_packets.saturating_sub(self.tx_at_reset);
            let dev = self.admin.get_basic_stats().ok();
            match dev {
                Some((tx, rx, _, _)) if tx > 0 || rx > 0 => {
                    ostd::early_println!(
                        "[kernel] ena: data path alive after reset (device tx {} rx {})",
                        tx,
                        rx
                    );
                }
                _ if sent_since > 0 => {
                    ostd::early_println!(
                        "[kernel] ena: data path dead after reset (host sent {}, device saw {:?}); rebooting the machine",
                        sent_since,
                        dev
                    );
                    ostd::power::restart(ostd::power::ExitCode::Failure);
                }
                _ => {}
            }
        }
        if self.debug_ticks > 0 {
            self.debug_ticks -= 1;
            let sts = self.admin.read32(crate::regs::DEV_STS);
            let ctl = self.admin.read32(crate::regs::DEV_CTL);
            let imask = self.admin.read32(crate::regs::INTR_MASK);
            let rx: Vec<(u16, u16, u16)> = self
                .rx
                .iter()
                .map(|r| (r.q.sq.tail, r.q.sq.next_to_comp, r.q.cq.head))
                .collect();
            let tx: Vec<(u16, u16, u16)> = self
                .tx
                .iter()
                .map(|t| (t.q.sq.tail, t.q.sq.next_to_comp, t.q.cq.head))
                .collect();
            let dev = self.admin.get_basic_stats().ok();
            ostd::early_println!(
                "[kernel] ena: dbg sts {:#x} ctl {:#x} imask {:#x} rx(tail,ntc,cqhead) {:?} tx {:?} host rx {} tx {} | device (tx,rx,rxdrop,txdrop) {:?}",
                sts,
                ctl,
                imask,
                rx,
                tx,
                self.stats.rx_packets,
                self.stats.tx_packets,
                dev
            );
        }
        for ev in self.admin.poll_aenq() {
            match ev {
                AenqEvent::KeepAlive { rx_drops, tx_drops } => {
                    self.last_keep_alive_ms = now;
                    self.stats.rx_drops_dev = rx_drops;
                    self.stats.tx_drops_dev = tx_drops;
                }
                AenqEvent::LinkChange { up } => {
                    if up != self.link_up {
                        ostd::early_println!(
                            "[kernel] ena: link {}",
                            if up { "up" } else { "down" }
                        );
                    }
                    self.link_up = up;
                }
                AenqEvent::FatalError => {
                    ostd::early_println!("[kernel] ena: device reported a fatal error");
                    self.request_reset("fatal error event");
                    return;
                }
                AenqEvent::Warning => ostd::warn!("device warning event"),
                AenqEvent::Notification(s) => ostd::debug!("device notification {}", s),
                AenqEvent::Unknown(g) => ostd::debug!("unknown AENQ group {}", g),
            }
        }
        if self.keep_alive_enabled
            && now.saturating_sub(self.last_keep_alive_ms) > KEEP_ALIVE_TIMEOUT_MS
        {
            ostd::early_println!(
                "[kernel] ena: no keep-alive for {} ms",
                now.saturating_sub(self.last_keep_alive_ms)
            );
            self.request_reset("keep-alive timeout");
            return;
        }
        if let Some(at) = self.test_reset_at_ms
            && now >= at
        {
            self.test_reset_at_ms = None;
            ostd::early_println!(
                "[kernel] ena: test reset requested (ena.test_reset); device stats {:?}",
                self.admin.get_basic_stats().ok()
            );
            self.request_reset("test");
        }
    }

    /// Schedules a reset for the next `receive()` (same softirq, but after the
    /// AENQ loop has finished with its borrow of `self`).
    fn request_reset(&mut self, why: &'static str) {
        if self.reset_pending.is_none() {
            self.reset_pending = Some(why);
            // Stop the watchdog from re-triggering while we wait for the softirq.
            self.last_keep_alive_ms = TICK_MS.load(Ordering::Relaxed);
        }
        aster_network::raise_receive_softirq();
    }

    /// Full device reset: admin queue, I/O queues, Rx buffers, RSS, AENQ.
    /// In-flight Tx buffers are dropped (the stack retransmits); the MAC and
    /// configuration are unchanged, so the interface keeps its address.
    fn reset(&mut self, why: &str) {
        self.resets += 1;
        let t0 = ostd::arch::read_tsc();
        ostd::early_println!(
            "[kernel] ena: resetting device ({}), reset #{}",
            why,
            self.resets
        );
        let n = self.rx.len().max(1);
        self.tx.clear();
        self.rx.clear();
        self.next_tx = 0;
        self.next_rx = 0;
        let r = self.admin.reinit().map_err(EnaError::Admin).and_then(|_| {
            // Same sequence as a fresh probe (Linux re-reads the device
            // attributes and queue limits after every reset too).
            let _ = self.admin.get_feature(admin::FEAT_DEVICE_ATTRIBUTES, 0);
            let _ = self.admin.get_feature(admin::FEAT_MAX_QUEUES_EXT, 1);
            let _ = Self::query_offloads(&mut self.admin);
            let _ = self
                .admin
                .set_feature(admin::FEAT_MTU, &[(MTU - 14) as u32]);
            self.setup_queues(n)
        });
        match r {
            Ok(()) => {
                self.setup_aenq();
                self.debug_ticks = 3;
                self.last_reset_ms = Some(TICK_MS.load(Ordering::Relaxed));
                self.tx_at_reset = self.stats.tx_packets;
                let _ = &self.msix;
                let freq = ostd::arch::tsc_freq().max(1);
                ostd::early_println!(
                    "[kernel] ena: reset done in {} ms, {} rx / {} tx queues",
                    ostd::arch::read_tsc().wrapping_sub(t0) / (freq / 1000).max(1),
                    self.rx.len(),
                    self.tx.len()
                );
            }
            Err(e) => {
                ostd::early_println!(
                    "[kernel] ena: reset failed: {:?}; retrying on the next tick",
                    e
                );
                self.last_keep_alive_ms = TICK_MS.load(Ordering::Relaxed); // avoid a tight loop
            }
        }
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
        self.reset_pending.is_some()
            || TICK_DUE.load(Ordering::Acquire)
            || self.rx.iter().any(|r| r.q.cq.has_entry(3, 0))
    }

    fn can_send(&self) -> bool {
        self.reset_pending.is_none()
            && self
                .tx
                .iter()
                .any(|t| t.q.sq.free_entries() >= 2 && !t.free_ids.is_empty())
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
        for txq in &mut self.tx {
            while let Some(cdesc) = txq.q.cq.pop::<TxCdesc>(3, 0) {
                let req_id = cdesc.req_id;
                if req_id >= IO_QUEUE_DEPTH || txq.bufs[req_id as usize].is_none() {
                    ostd::error!("tx completion with bad req_id {}", req_id);
                    continue;
                }
                txq.bufs[req_id as usize] = None;
                txq.free_ids.push(req_id);
                // Like Linux (`ena_com_comp_ack`): count the descriptors of the
                // completed request; `sq_head_idx` in the completion is not
                // reliable across all device generations.
                let n = txq.descs[req_id as usize].max(1) as u16;
                txq.q.sq.next_to_comp = txq.q.sq.next_to_comp.wrapping_add(n);
            }
        }
    }

    fn notify_poll_end(&mut self) {
        for qi in 0..self.rx.len() {
            self.flush_rx_doorbell(qi);
        }
        self.report_cq_heads();
        for qi in 0..self.rx.len() {
            self.unmask_io_irq(qi);
        }
    }
}

impl Debug for EnaDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EnaDevice")
            .field("mac", &self.mac)
            .field("queues", &self.rx.len())
            .field("offloads", &self.offloads)
            .field("link_up", &self.link_up)
            .field("resets", &self.resets)
            .field("stats", &self.stats)
            .finish()
    }
}

/// Per-tick bookkeeping shared with `lib.rs`.
pub(crate) static TICKS: AtomicU32 = AtomicU32::new(0);
