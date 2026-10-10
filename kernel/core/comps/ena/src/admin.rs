// SPDX-License-Identifier: MPL-2.0

//! The ENA admin queue: device reset, readless MMIO, admin commands
//! (`GET_FEATURE`, `SET_FEATURE`, `CREATE_CQ`, `CREATE_SQ`) in polling mode,
//! and the asynchronous event queue (AENQ) the device requires to exist.
//!
//! Reference: `ena_com.c` and `ena_admin_defs.h` in Linux.

use core::{
    hint::spin_loop,
    sync::atomic::{Ordering, fence},
};

use aster_pci::cfg_space::BarAccess;
use ostd::{
    Error,
    mm::{HasDaddr, HasSize, VmIo, VmIoOnce, dma::DmaCoherent},
};

use crate::regs;

pub(crate) const ADMIN_QUEUE_DEPTH: u16 = 32;
pub(crate) const AENQ_DEPTH: u16 = 16;

// Opcodes.
pub(crate) const OP_CREATE_SQ: u8 = 1;
pub(crate) const OP_CREATE_CQ: u8 = 3;
pub(crate) const OP_GET_FEATURE: u8 = 8;
pub(crate) const OP_GET_STATS: u8 = 11;
pub(crate) const OP_SET_FEATURE: u8 = 9;

// Feature ids.
pub(crate) const FEAT_DEVICE_ATTRIBUTES: u8 = 1;
pub(crate) const FEAT_MAX_QUEUES_NUM: u8 = 2;
pub(crate) const FEAT_MAX_QUEUES_EXT: u8 = 7;
pub(crate) const FEAT_RSS_HASH_FUNCTION: u8 = 10;
pub(crate) const FEAT_STATELESS_OFFLOAD_CONFIG: u8 = 11;
pub(crate) const FEAT_RSS_INDIRECTION_TABLE: u8 = 12;
pub(crate) const FEAT_MTU: u8 = 14;
pub(crate) const FEAT_RSS_HASH_INPUT: u8 = 18;
pub(crate) const FEAT_AENQ_CONFIG: u8 = 26;
pub(crate) const FEAT_HOST_ATTR_CONFIG: u8 = 28;

// AENQ groups (bit positions) and the matching syndrome values.
pub(crate) const AENQ_GROUP_LINK_CHANGE: u32 = 1 << 0;
pub(crate) const AENQ_GROUP_FATAL_ERROR: u32 = 1 << 1;
pub(crate) const AENQ_GROUP_WARNING: u32 = 1 << 2;
pub(crate) const AENQ_GROUP_NOTIFICATION: u32 = 1 << 3;
pub(crate) const AENQ_GROUP_KEEP_ALIVE: u32 = 1 << 4;

/// Flag in `AqEntry::flags`: the command's data is in the control buffer.
const AQ_CTRL_DATA_INDIRECT: u8 = 1 << 2;

pub(crate) const SQ_DIRECTION_TX: u8 = 1;
pub(crate) const SQ_DIRECTION_RX: u8 = 2;
pub(crate) const PLACEMENT_POLICY_HOST: u8 = 1;
pub(crate) const COMPLETION_POLICY_DESC: u8 = 0;

const AQ_PHASE_MASK: u8 = 0x1;
const ACQ_PHASE_MASK: u8 = 0x1;
const COMMAND_ID_MASK: u16 = 0x0fff;

/// `struct ena_admin_aq_entry` (64 bytes). The command-specific fields are
/// laid out in `words` by the builders below.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct AqEntry {
    pub(crate) command_id: u16,
    pub(crate) opcode: u8,
    pub(crate) flags: u8,
    pub(crate) words: [u32; 15],
}

/// `struct ena_admin_acq_entry` (64 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct AcqEntry {
    pub(crate) command: u16,
    pub(crate) status: u8,
    pub(crate) flags: u8,
    pub(crate) extended_status: u16,
    pub(crate) sq_head_indx: u16,
    pub(crate) data: [u32; 14],
}

/// `struct ena_admin_aenq_entry` (64 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct AenqEntry {
    pub(crate) group: u16,
    pub(crate) syndrome: u16,
    pub(crate) flags: u8,
    pub(crate) reserved1: [u8; 3],
    pub(crate) timestamp_low: u32,
    pub(crate) timestamp_high: u32,
    pub(crate) data: [u32; 12],
}

/// An asynchronous event read from the AENQ.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AenqEvent {
    LinkChange { up: bool },
    FatalError,
    Warning,
    Notification(u16),
    KeepAlive { rx_drops: u64, tx_drops: u64 },
    Unknown(u16),
}

/// `struct ena_admin_ena_mmio_req_read_less_resp`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct MmioReadResp {
    req_id: u16,
    reg_off: u16,
    reg_val: u32,
}

#[derive(Clone, Copy, Debug)]
#[expect(dead_code)] // the payloads are for `{:?}` in error messages
pub(crate) enum AdminError {
    /// The device did not become ready / finish the reset in time.
    Timeout,
    /// The device reported `status` for an admin command.
    CommandFailed(u8),
    /// The device is not ready (`DEV_STS.READY` clear).
    NotReady,
    /// DMA memory could not be allocated.
    NoMemory(Error),
}

impl From<Error> for AdminError {
    fn from(e: Error) -> Self {
        AdminError::NoMemory(e)
    }
}

/// Busy-waits for `micros` microseconds using the TSC, or a bounded spin when
/// the TSC frequency is not known yet.
pub(crate) fn delay_us(micros: u64) {
    let freq = ostd::arch::tsc_freq();
    if freq == 0 {
        for _ in 0..micros * 200 {
            spin_loop();
        }
        return;
    }
    let start = ostd::arch::read_tsc();
    let ticks = freq / 1_000_000 * micros;
    while ostd::arch::read_tsc().wrapping_sub(start) < ticks {
        spin_loop();
    }
}

fn mem_addr_words(daddr: usize) -> (u32, u32) {
    (daddr as u32, ((daddr >> 32) & 0xffff) as u32)
}

pub(crate) struct AdminQueue {
    bar: BarAccess,
    sq: DmaCoherent,
    cq: DmaCoherent,
    aenq: DmaCoherent,
    mmio_resp: DmaCoherent,
    sq_tail: u16,
    sq_phase: u8,
    cq_head: u16,
    cq_phase: u8,
    next_cmd_id: u16,
    mmio_seq: u16,
    readless: bool,
    /// Admin command timeout in microseconds (from `CAPS.ADMIN_CMD_TO`).
    cmd_timeout_us: u64,
    aenq_head: u16,
    aenq_phase: u8,
    /// Scratch control buffer for indirect GET/SET_FEATURE (one page).
    ctrl_buf: DmaCoherent,
    /// Host info page (`ena_admin_host_info`) handed to the device.
    host_info: DmaCoherent,
}

impl AdminQueue {
    /// Allocates the queues, resets the device and programs the admin queue
    /// registers. On return the device is ready for admin commands.
    pub(crate) fn new(bar: BarAccess) -> Result<Self, AdminError> {
        let sq = DmaCoherent::alloc(1, true)?;
        let cq = DmaCoherent::alloc(1, true)?;
        let aenq = DmaCoherent::alloc(1, true)?;
        let mmio_resp = DmaCoherent::alloc(1, true)?;
        let ctrl_buf = DmaCoherent::alloc(1, true)?;
        let host_info = DmaCoherent::alloc(1, true)?;
        let mut this = Self {
            bar,
            sq,
            cq,
            aenq,
            mmio_resp,
            sq_tail: 0,
            sq_phase: 1,
            cq_head: 0,
            cq_phase: 1,
            next_cmd_id: 0,
            mmio_seq: 0,
            readless: true,
            cmd_timeout_us: 3_000_000,
            aenq_head: AENQ_DEPTH,
            aenq_phase: 1,
            ctrl_buf,
            host_info,
        };
        this.write_mmio_resp_addr();
        this.reset()?;
        this.init_queues()?;
        Ok(this)
    }

    /// Resets the device and re-programs the admin queue; every I/O queue
    /// created before is gone afterwards. Used for recovery after a fatal
    /// error or a missed keep-alive.
    pub(crate) fn reinit(&mut self) -> Result<(), AdminError> {
        self.sq_tail = 0;
        self.sq_phase = 1;
        self.cq_head = 0;
        self.cq_phase = 1;
        self.next_cmd_id = 0;
        self.aenq_head = AENQ_DEPTH;
        self.aenq_phase = 1;
        self.readless = true;
        // Zero the rings so stale phase bits cannot be mistaken for completions.
        for mem in [&self.sq, &self.cq, &self.aenq] {
            let zero = [0u8; 64];
            for i in 0..(mem.size() / 64) {
                mem.write_bytes(i * 64, &zero).unwrap();
            }
        }
        self.write_mmio_resp_addr();
        self.reset()?;
        self.init_queues()
    }

    /// `SET_FEATURE HOST_ATTR_CONFIG`: tells the device who the driver is.
    /// Linux does this before anything else. Tried here and reverted: with
    /// host attributes set (OS type FreeBSD), the Nitro device stopped
    /// delivering unicast Rx from outside the VPC while DHCP/DNS still worked.
    /// Kept for experiments; not called.
    #[expect(dead_code)]
    pub(crate) fn set_host_attributes(&mut self) -> Result<(), AdminError> {
        // struct ena_admin_host_info: os_type(u32) os_dist_str[128] os_dist(u32)
        // kernel_ver_str[32] kernel_ver(u32) driver_version(u32)
        // supported_network_features[2] ena_spec_version(u16) bdf(u16) num_cpus(u16) reserved(u16) driver_supported_features(u32)
        let mut page = alloc::vec![0u8; 4096];
        // OS type: there is no "other"; 4 (FreeBSD) is the closest to a
        // from-scratch driver and does not make the device assume Linux quirks.
        page[0..4].copy_from_slice(&4u32.to_le_bytes());
        let dist = b"Asterinas/elixir_unikernel";
        page[4..4 + dist.len()].copy_from_slice(dist);
        let kver = b"asterinas";
        page[136..136 + kver.len()].copy_from_slice(kver);
        page[172..176].copy_from_slice(&(1u32 << 24 | 1).to_le_bytes()); // driver version 1.0.1 (major 1, minor 0, sub 1)
        page[184..186].copy_from_slice(&(2u16 << 8).to_le_bytes()); // ENA spec version 2.0
        page[188..190].copy_from_slice(&(ostd::cpu::num_cpus() as u16).to_le_bytes());
        self.host_info.write_bytes(0, &page).unwrap();
        fence(Ordering::SeqCst);
        let (lo, hi) = mem_addr_words(self.host_info.daddr());
        // host_attr desc: os_info_ba (lo, hi), debug_ba (0, 0), debug_area_size 0
        self.set_feature(FEAT_HOST_ATTR_CONFIG, &[lo, hi, 0, 0, 0])
    }

    /// Enables the given AENQ groups (`SET_FEATURE AENQ_CONFIG`) and starts
    /// event delivery by writing the AENQ head doorbell (= depth: every entry
    /// available); the device writes no events before that. The AENQ is
    /// polled every 100 ms from the timer tick, with the admin interrupt
    /// masked: keep-alives come once a second and the watchdog allows 6 s, so
    /// polling is plenty, and leaving vector 0 unmasked hung the guest after a
    /// device reset (the interrupt re-fires until the head doorbell is
    /// written, which the tick does too late). `ena.aenq_irq=1` unmasks it
    /// for experiments.
    pub(crate) fn enable_aenq_groups(&mut self, groups: u32) -> Result<u32, AdminError> {
        let supported = self.get_feature(FEAT_AENQ_CONFIG, 0)?[0];
        let enabled = groups & supported;
        self.set_feature(FEAT_AENQ_CONFIG, &[supported, enabled])?;
        if crate::AENQ_IRQ_PARAM.get().is_some_and(|v| v == "1") {
            self.write32(regs::INTR_MASK, 0);
        } else {
            self.write32(regs::INTR_MASK, regs::ADMIN_INTR_MASK);
        }
        self.write32(regs::AENQ_HEAD_DB, AENQ_DEPTH as u32);
        Ok(enabled)
    }

    /// Reads all pending asynchronous events and acknowledges them.
    pub(crate) fn poll_aenq(&mut self) -> alloc::vec::Vec<AenqEvent> {
        const AENQ_PHASE_MASK: u8 = 0x1;
        let mut events = alloc::vec::Vec::new();
        loop {
            let slot = (self.aenq_head % AENQ_DEPTH) as usize * size_of::<AenqEntry>();
            let flags: u8 = self.aenq.read_once(slot + 4).unwrap();
            if flags & AENQ_PHASE_MASK != self.aenq_phase {
                break;
            }
            fence(Ordering::SeqCst);
            let e: AenqEntry = self.aenq.read_val(slot).unwrap();
            self.aenq_head = self.aenq_head.wrapping_add(1);
            if self.aenq_head.is_multiple_of(AENQ_DEPTH) {
                self.aenq_phase ^= 1;
            }
            events.push(match e.group {
                0 => AenqEvent::LinkChange {
                    up: e.data[0] & 1 != 0,
                },
                1 => AenqEvent::FatalError,
                2 => AenqEvent::Warning,
                3 => AenqEvent::Notification(e.syndrome),
                4 => AenqEvent::KeepAlive {
                    rx_drops: e.data[0] as u64 | (e.data[1] as u64) << 32,
                    tx_drops: e.data[2] as u64 | (e.data[3] as u64) << 32,
                },
                g => AenqEvent::Unknown(g),
            });
            if events.len() >= AENQ_DEPTH as usize {
                break;
            }
        }
        if !events.is_empty() {
            fence(Ordering::SeqCst);
            self.write32(regs::AENQ_HEAD_DB, self.aenq_head as u32);
        }
        events
    }

    /// Reads `DEV_STS` and reports whether the device flagged a fatal error.
    /// (Fatal errors arrive through the AENQ as well; this is for diagnostics.)
    #[expect(dead_code)]
    pub(crate) fn fatal_error(&mut self) -> bool {
        let sts = self.read32(regs::DEV_STS);
        sts != regs::MMIO_READ_TIMEOUT && sts & regs::DEV_STS_FATAL_ERROR != 0
    }

    /// `GET_FEATURE` whose response goes to the control buffer; returns up to
    /// `len` bytes of it.
    #[expect(dead_code)]
    pub(crate) fn get_feature_indirect(
        &mut self,
        feature_id: u8,
        version: u8,
        len: usize,
    ) -> Result<alloc::vec::Vec<u8>, AdminError> {
        let mut cmd = AqEntry {
            opcode: OP_GET_FEATURE,
            flags: AQ_CTRL_DATA_INDIRECT,
            ..Default::default()
        };
        let (lo, hi) = mem_addr_words(self.ctrl_buf.daddr());
        cmd.words[0] = len as u32;
        cmd.words[1] = lo;
        cmd.words[2] = hi;
        cmd.words[3] = (feature_id as u32) << 8 | (version as u32) << 16;
        self.execute(cmd)?;
        let mut out = alloc::vec![0u8; len];
        self.ctrl_buf.read_bytes(0, &mut out).unwrap();
        Ok(out)
    }

    /// `SET_FEATURE` with `inline_data` plus `ctrl` bytes in the control buffer.
    pub(crate) fn set_feature_indirect(
        &mut self,
        feature_id: u8,
        inline_data: &[u32],
        ctrl: &[u8],
    ) -> Result<(), AdminError> {
        assert!(ctrl.len() <= self.ctrl_buf.size());
        self.ctrl_buf.write_bytes(0, ctrl).unwrap();
        fence(Ordering::SeqCst);
        let mut cmd = AqEntry {
            opcode: OP_SET_FEATURE,
            flags: AQ_CTRL_DATA_INDIRECT,
            ..Default::default()
        };
        let (lo, hi) = mem_addr_words(self.ctrl_buf.daddr());
        cmd.words[0] = ctrl.len() as u32;
        cmd.words[1] = lo;
        cmd.words[2] = hi;
        cmd.words[3] = (feature_id as u32) << 8;
        for (i, w) in inline_data.iter().enumerate().take(11) {
            cmd.words[4 + i] = *w;
        }
        self.execute(cmd).map(|_| ())
    }

    fn write32(&self, off: usize, val: u32) {
        self.bar.write_once(off, val).unwrap();
    }

    fn write_mmio_resp_addr(&self) {
        let (lo, hi) = mem_addr_words(self.mmio_resp.daddr());
        self.write32(regs::MMIO_RESP_LO, lo);
        self.write32(regs::MMIO_RESP_HI, hi);
    }

    /// Reads a BAR0 register. ENA devices answer register reads through a DMA
    /// write into `mmio_resp` ("readless"); a direct read is the fallback.
    pub(crate) fn read32(&mut self, off: usize) -> u32 {
        if !self.readless {
            return self.bar.read_once(off).unwrap();
        }
        self.mmio_seq = self.mmio_seq.wrapping_add(1);
        let seq = self.mmio_seq;
        // Invalidate the previous answer so a stale one is never accepted.
        self.mmio_resp
            .write_val(
                0,
                &MmioReadResp {
                    req_id: seq.wrapping_add(0xdead),
                    reg_off: 0,
                    reg_val: 0,
                },
            )
            .unwrap();
        fence(Ordering::SeqCst);
        let req = ((off as u32) << regs::MMIO_REG_READ_REG_OFF_SHIFT)
            | (seq as u32 & regs::MMIO_REG_READ_REQ_ID_MASK);
        self.write32(regs::MMIO_REG_READ, req);
        for _ in 0..200_000 {
            let resp: MmioReadResp = self.mmio_resp.read_val(0).unwrap();
            if resp.req_id == seq {
                if resp.reg_off as usize != off {
                    ostd::warn!(
                        "readless MMIO answered for offset {:#x}, wanted {:#x}",
                        resp.reg_off,
                        off
                    );
                    return regs::MMIO_READ_TIMEOUT;
                }
                return resp.reg_val;
            }
            delay_us(1);
        }
        ostd::warn!(
            "readless MMIO read of {:#x} timed out; using direct reads",
            off
        );
        self.readless = false;
        self.bar.read_once(off).unwrap()
    }

    fn wait_reset_state(
        &mut self,
        timeout_100ms: u32,
        expect_in_progress: bool,
    ) -> Result<(), AdminError> {
        for _ in 0..timeout_100ms.max(1) * 100 {
            let sts = self.read32(regs::DEV_STS);
            if sts == regs::MMIO_READ_TIMEOUT {
                return Err(AdminError::Timeout);
            }
            if (sts & regs::DEV_STS_RESET_IN_PROGRESS != 0) == expect_in_progress {
                return Ok(());
            }
            delay_us(1_000);
        }
        Err(AdminError::Timeout)
    }

    fn reset(&mut self) -> Result<(), AdminError> {
        let sts = self.read32(regs::DEV_STS);
        let caps = self.read32(regs::CAPS);
        if sts == regs::MMIO_READ_TIMEOUT || caps == regs::MMIO_READ_TIMEOUT {
            return Err(AdminError::Timeout);
        }
        if sts & regs::DEV_STS_READY == 0 {
            return Err(AdminError::NotReady);
        }
        let timeout = (caps & regs::CAPS_RESET_TIMEOUT_MASK) >> regs::CAPS_RESET_TIMEOUT_SHIFT;
        ostd::debug!(
            "caps {:#x}: reset timeout {} x100ms, dma width {}",
            caps,
            timeout,
            (caps & regs::CAPS_DMA_ADDR_WIDTH_MASK) >> regs::CAPS_DMA_ADDR_WIDTH_SHIFT
        );

        self.write32(
            regs::DEV_CTL,
            regs::DEV_CTL_DEV_RESET
                | (regs::RESET_REASON_NORMAL << regs::DEV_CTL_RESET_REASON_SHIFT),
        );
        // The reset clears the readless response address; write it again.
        self.write_mmio_resp_addr();
        self.wait_reset_state(timeout, true)?;
        self.write32(regs::DEV_CTL, 0);
        self.wait_reset_state(timeout, false)?;

        let to = (caps & regs::CAPS_ADMIN_CMD_TO_MASK) >> regs::CAPS_ADMIN_CMD_TO_SHIFT;
        if to != 0 {
            self.cmd_timeout_us = to as u64 * 100_000;
        }
        Ok(())
    }

    fn init_queues(&mut self) -> Result<(), AdminError> {
        let sts = self.read32(regs::DEV_STS);
        if sts & regs::DEV_STS_READY == 0 {
            return Err(AdminError::NotReady);
        }
        let (lo, hi) = mem_addr_words(self.sq.daddr());
        self.write32(regs::AQ_BASE_LO, lo);
        self.write32(regs::AQ_BASE_HI, hi);
        let (lo, hi) = mem_addr_words(self.cq.daddr());
        self.write32(regs::ACQ_BASE_LO, lo);
        self.write32(regs::ACQ_BASE_HI, hi);
        let entry = (size_of::<AqEntry>() as u32) << regs::AQ_CAPS_ENTRY_SIZE_SHIFT;
        self.write32(
            regs::AQ_CAPS,
            entry | (ADMIN_QUEUE_DEPTH as u32 & regs::AQ_CAPS_DEPTH_MASK),
        );
        self.write32(
            regs::ACQ_CAPS,
            entry | (ADMIN_QUEUE_DEPTH as u32 & regs::AQ_CAPS_DEPTH_MASK),
        );

        let (lo, hi) = mem_addr_words(self.aenq.daddr());
        self.write32(regs::AENQ_BASE_LO, lo);
        self.write32(regs::AENQ_BASE_HI, hi);
        self.write32(
            regs::AENQ_CAPS,
            (64u32 << regs::AQ_CAPS_ENTRY_SIZE_SHIFT) | AENQ_DEPTH as u32,
        );

        // Admin completions are polled: mask the admin interrupt.
        self.write32(regs::INTR_MASK, regs::ADMIN_INTR_MASK);
        Ok(())
    }

    /// Submits `cmd` and waits for its completion. Returns the completion
    /// entry; `status != 0` is reported as `CommandFailed`.
    pub(crate) fn execute(&mut self, mut cmd: AqEntry) -> Result<AcqEntry, AdminError> {
        let depth = ADMIN_QUEUE_DEPTH;
        let cmd_id = self.next_cmd_id;
        self.next_cmd_id = (self.next_cmd_id + 1) % depth;
        cmd.command_id = (cmd.command_id & !COMMAND_ID_MASK) | (cmd_id & COMMAND_ID_MASK);
        cmd.flags = (cmd.flags & !AQ_PHASE_MASK) | (self.sq_phase & AQ_PHASE_MASK);

        let slot = (self.sq_tail % depth) as usize * size_of::<AqEntry>();
        self.sq.write_val(slot, &cmd).unwrap();
        fence(Ordering::SeqCst);
        self.sq_tail = self.sq_tail.wrapping_add(1);
        if self.sq_tail.is_multiple_of(depth) {
            self.sq_phase ^= 1;
        }
        self.write32(regs::AQ_DB, self.sq_tail as u32);

        let mut waited = 0u64;
        loop {
            let slot = (self.cq_head % depth) as usize * size_of::<AcqEntry>();
            let flags: u8 = self.cq.read_once(slot + 3).unwrap();
            if flags & ACQ_PHASE_MASK == self.cq_phase {
                fence(Ordering::SeqCst);
                let entry: AcqEntry = self.cq.read_val(slot).unwrap();
                self.cq_head = self.cq_head.wrapping_add(1);
                if self.cq_head.is_multiple_of(depth) {
                    self.cq_phase ^= 1;
                }
                if entry.command & COMMAND_ID_MASK != cmd_id {
                    ostd::warn!(
                        "admin completion for command {} while waiting for {}",
                        entry.command & COMMAND_ID_MASK,
                        cmd_id
                    );
                    continue;
                }
                ostd::debug!(
                    "admin op {} -> status {} ext {}",
                    cmd.opcode,
                    entry.status,
                    entry.extended_status
                );
                if entry.status != 0 {
                    return Err(AdminError::CommandFailed(entry.status));
                }
                return Ok(entry);
            }
            if waited >= self.cmd_timeout_us {
                ostd::error!("admin op {} timed out after {} us", cmd.opcode, waited);
                return Err(AdminError::Timeout);
            }
            delay_us(20);
            waited += 20;
        }
    }

    /// `GET_STATS` basic, device-wide: `(tx_pkts, rx_pkts, rx_drops, tx_drops)`.
    pub(crate) fn get_basic_stats(&mut self) -> Result<(u64, u64, u64, u64), AdminError> {
        let mut cmd = AqEntry {
            opcode: OP_GET_STATS,
            ..Default::default()
        };
        // words[3]: type (u8 basic=0), scope (u8 eth_traffic=1), reserved; words[4]: queue_idx | device_id<<16 (0xffff = all)
        cmd.words[3] = 1 << 8;
        cmd.words[4] = 0xffff << 16;
        let d = self.execute(cmd)?.data;
        let u64at = |i: usize| d[i] as u64 | (d[i + 1] as u64) << 32;
        Ok((u64at(2), u64at(6), u64at(8), u64at(10)))
    }

    /// `GET_FEATURE feature_id` (inline response).
    pub(crate) fn get_feature(
        &mut self,
        feature_id: u8,
        version: u8,
    ) -> Result<[u32; 14], AdminError> {
        let mut cmd = AqEntry {
            opcode: OP_GET_FEATURE,
            ..Default::default()
        };
        // words[0..3] = control buffer (unused), words[3] = feat_common
        cmd.words[3] = (feature_id as u32) << 8 | (version as u32) << 16;
        Ok(self.execute(cmd)?.data)
    }

    /// `SET_FEATURE feature_id` with up to 11 words of inline data.
    pub(crate) fn set_feature(&mut self, feature_id: u8, data: &[u32]) -> Result<(), AdminError> {
        let mut cmd = AqEntry {
            opcode: OP_SET_FEATURE,
            ..Default::default()
        };
        cmd.words[3] = (feature_id as u32) << 8;
        for (i, w) in data.iter().enumerate().take(11) {
            cmd.words[4 + i] = *w;
        }
        self.execute(cmd).map(|_| ())
    }

    /// `CREATE_CQ`. Returns `(cq_idx, head_db_offset, unmask_offset)`.
    pub(crate) fn create_cq(
        &mut self,
        depth: u16,
        cdesc_size: u16,
        msix_vector: u32,
        base: usize,
    ) -> Result<(u16, u32, u32), AdminError> {
        const INTERRUPT_MODE_ENABLED: u32 = 1 << 5;
        let mut cmd = AqEntry {
            opcode: OP_CREATE_CQ,
            ..Default::default()
        };
        let cq_caps_1 = INTERRUPT_MODE_ENABLED;
        let cq_caps_2 = (cdesc_size / 4) as u32 & 0x1f;
        cmd.words[0] = cq_caps_1 | cq_caps_2 << 8 | (depth as u32) << 16;
        cmd.words[1] = msix_vector;
        let (lo, hi) = mem_addr_words(base);
        cmd.words[2] = lo;
        cmd.words[3] = hi;
        let resp = self.execute(cmd)?;
        let cq_idx = (resp.data[0] & 0xffff) as u16;
        let actual_depth = (resp.data[0] >> 16) as u16;
        ostd::debug!(
            "created cq {} depth {} head_db {:#x} unmask {:#x}",
            cq_idx,
            actual_depth,
            resp.data[2],
            resp.data[3]
        );
        Ok((cq_idx, resp.data[2], resp.data[3]))
    }

    /// `CREATE_SQ`. Returns `(sq_idx, doorbell_offset)`.
    pub(crate) fn create_sq(
        &mut self,
        direction: u8,
        cq_idx: u16,
        depth: u16,
        base: usize,
    ) -> Result<(u16, u32), AdminError> {
        const IS_PHYSICALLY_CONTIGUOUS: u32 = 1;
        let mut cmd = AqEntry {
            opcode: OP_CREATE_SQ,
            ..Default::default()
        };
        let sq_identity = ((direction as u32) << 5) & 0xe0;
        let sq_caps_2 =
            (PLACEMENT_POLICY_HOST as u32 & 0xf) | ((COMPLETION_POLICY_DESC as u32) << 4 & 0x70);
        let sq_caps_3 = IS_PHYSICALLY_CONTIGUOUS;
        cmd.words[0] = sq_identity | sq_caps_2 << 16 | sq_caps_3 << 24;
        cmd.words[1] = cq_idx as u32 | (depth as u32) << 16;
        let (lo, hi) = mem_addr_words(base);
        cmd.words[2] = lo;
        cmd.words[3] = hi;
        let resp = self.execute(cmd)?;
        let sq_idx = (resp.data[0] & 0xffff) as u16;
        ostd::debug!(
            "created sq {} (dir {}) doorbell {:#x}",
            sq_idx,
            direction,
            resp.data[1]
        );
        Ok((sq_idx, resp.data[1]))
    }

    pub(crate) fn bar(&self) -> &BarAccess {
        &self.bar
    }
}
