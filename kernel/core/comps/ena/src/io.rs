// SPDX-License-Identifier: MPL-2.0

//! ENA I/O rings: one Tx submission/completion pair and one Rx pair, in
//! host memory, with descriptor-based completion.
//!
//! Reference: `ena_eth_io_defs.h` and `ena_eth_com.c` in Linux.

use core::sync::atomic::{Ordering, fence};

use ostd::mm::{HasDaddr, VmIo, VmIoOnce, dma::DmaCoherent};
use ostd_pod::Pod;

/// Depth of every I/O ring. Must be a power of two and at most the device
/// maximum (1024 on current ENA devices).
pub(crate) const IO_QUEUE_DEPTH: u16 = 128;

/// `struct ena_eth_io_tx_desc`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct TxDesc {
    pub(crate) len_ctrl: u32,
    pub(crate) meta_ctrl: u32,
    pub(crate) buff_addr_lo: u32,
    pub(crate) buff_addr_hi_hdr_sz: u32,
}

pub(crate) const TX_DESC_LENGTH_MASK: u32 = 0xffff;
pub(crate) const TX_DESC_REQ_ID_HI_SHIFT: u32 = 16;
pub(crate) const TX_DESC_PHASE_SHIFT: u32 = 24;
pub(crate) const TX_DESC_FIRST: u32 = 1 << 26;
pub(crate) const TX_DESC_LAST: u32 = 1 << 27;
pub(crate) const TX_DESC_COMP_REQ: u32 = 1 << 28;
pub(crate) const TX_DESC_REQ_ID_LO_SHIFT: u32 = 22;
pub(crate) const TX_DESC_ADDR_HI_MASK: u32 = 0xffff;

/// `struct ena_eth_io_tx_cdesc` (8 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct TxCdesc {
    pub(crate) req_id: u16,
    pub(crate) status: u8,
    pub(crate) flags: u8,
    pub(crate) sub_qid: u16,
    pub(crate) sq_head_idx: u16,
}

/// `struct ena_eth_io_rx_desc`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct RxDesc {
    pub(crate) length: u16,
    pub(crate) reserved2: u8,
    pub(crate) ctrl: u8,
    pub(crate) req_id: u16,
    pub(crate) reserved6: u16,
    pub(crate) buff_addr_lo: u32,
    pub(crate) buff_addr_hi: u16,
    pub(crate) reserved16_w3: u16,
}

pub(crate) const RX_DESC_PHASE_MASK: u8 = 0x1;
pub(crate) const RX_DESC_FIRST: u8 = 1 << 2;
pub(crate) const RX_DESC_LAST: u8 = 1 << 3;
pub(crate) const RX_DESC_COMP_REQ: u8 = 1 << 4;

/// `struct ena_eth_io_rx_cdesc_base` (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct RxCdesc {
    pub(crate) status: u32,
    pub(crate) length: u16,
    pub(crate) req_id: u16,
    pub(crate) hash: u32,
    pub(crate) sub_qid: u16,
    pub(crate) offset: u8,
    pub(crate) reserved: u8,
}

/// The phase bit is bit 24 of `status`, i.e. bit 0 of its fourth byte.
pub(crate) const RX_CDESC_FIRST: u32 = 1 << 26;
pub(crate) const RX_CDESC_LAST: u32 = 1 << 27;

/// `struct ena_eth_io_intr_reg`: unmask with zero delay.
pub(crate) const INTR_UNMASK: u32 = 1 << 30;

/// A submission ring of `IO_QUEUE_DEPTH` 16-byte descriptors.
pub(crate) struct SubmissionRing {
    mem: DmaCoherent,
    pub(crate) tail: u16,
    pub(crate) next_to_comp: u16,
    pub(crate) phase: u8,
}

impl SubmissionRing {
    pub(crate) fn new() -> Result<Self, ostd::Error> {
        let bytes = IO_QUEUE_DEPTH as usize * 16;
        let mem = DmaCoherent::alloc(bytes.div_ceil(ostd::mm::PAGE_SIZE), true)?;
        Ok(Self {
            mem,
            tail: 0,
            next_to_comp: 0,
            phase: 1,
        })
    }

    pub(crate) fn daddr(&self) -> usize {
        self.mem.daddr()
    }

    pub(crate) fn free_entries(&self) -> u16 {
        IO_QUEUE_DEPTH - 1 - self.tail.wrapping_sub(self.next_to_comp)
    }

    /// Writes `desc` at the tail and advances it (flipping the phase on wrap).
    pub(crate) fn push<T: Pod>(&mut self, desc: &T) {
        let slot = (self.tail % IO_QUEUE_DEPTH) as usize * 16;
        self.mem.write_val(slot, desc).unwrap();
        self.tail = self.tail.wrapping_add(1);
        if self.tail.is_multiple_of(IO_QUEUE_DEPTH) {
            self.phase ^= 1;
        }
    }
}

/// A completion ring of `IO_QUEUE_DEPTH` entries of `entry_size` bytes.
pub(crate) struct CompletionRing {
    mem: DmaCoherent,
    entry_size: usize,
    pub(crate) head: u16,
    pub(crate) phase: u8,
    pub(crate) last_head_reported: u16,
}

impl CompletionRing {
    pub(crate) fn new(entry_size: usize) -> Result<Self, ostd::Error> {
        let bytes = IO_QUEUE_DEPTH as usize * entry_size;
        let mem = DmaCoherent::alloc(bytes.div_ceil(ostd::mm::PAGE_SIZE), true)?;
        Ok(Self {
            mem,
            entry_size,
            head: 0,
            phase: 1,
            last_head_reported: 0,
        })
    }

    pub(crate) fn daddr(&self) -> usize {
        self.mem.daddr()
    }

    /// Whether the device has written the entry at the head (phase bit at
    /// `phase_byte`, bit `phase_bit` matches).
    pub(crate) fn has_entry(&self, phase_byte: usize, phase_bit: u8) -> bool {
        let slot = (self.head % IO_QUEUE_DEPTH) as usize * self.entry_size;
        let b: u8 = self.mem.read_once(slot + phase_byte).unwrap();
        (b >> phase_bit) & 1 == self.phase
    }

    /// Returns the entry at the head if the device has written it, and
    /// advances the head.
    pub(crate) fn pop<T: Pod>(&mut self, phase_byte: usize, phase_bit: u8) -> Option<T> {
        if !self.has_entry(phase_byte, phase_bit) {
            return None;
        }
        let slot = (self.head % IO_QUEUE_DEPTH) as usize * self.entry_size;
        fence(Ordering::SeqCst);
        let entry: T = self.mem.read_val(slot).unwrap();
        self.head = self.head.wrapping_add(1);
        if self.head.is_multiple_of(IO_QUEUE_DEPTH) {
            self.phase ^= 1;
        }
        Some(entry)
    }
}
