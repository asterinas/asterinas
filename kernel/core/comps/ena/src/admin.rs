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
    mm::{HasDaddr, VmIo, VmIoOnce, dma::DmaCoherent},
};

use crate::regs;

pub(crate) const ADMIN_QUEUE_DEPTH: u16 = 32;
pub(crate) const AENQ_DEPTH: u16 = 16;

// Opcodes.
pub(crate) const OP_CREATE_SQ: u8 = 1;
pub(crate) const OP_CREATE_CQ: u8 = 3;
pub(crate) const OP_GET_FEATURE: u8 = 8;
pub(crate) const OP_SET_FEATURE: u8 = 9;

// Feature ids.
pub(crate) const FEAT_DEVICE_ATTRIBUTES: u8 = 1;
pub(crate) const FEAT_MAX_QUEUES_NUM: u8 = 2;
pub(crate) const FEAT_MAX_QUEUES_EXT: u8 = 7;
pub(crate) const FEAT_MTU: u8 = 14;
pub(crate) const FEAT_AENQ_CONFIG: u8 = 26;

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
}

impl AdminQueue {
    /// Allocates the queues, resets the device and programs the admin queue
    /// registers. On return the device is ready for admin commands.
    pub(crate) fn new(bar: BarAccess) -> Result<Self, AdminError> {
        let sq = DmaCoherent::alloc(1, true)?;
        let cq = DmaCoherent::alloc(1, true)?;
        let aenq = DmaCoherent::alloc(1, true)?;
        let mmio_resp = DmaCoherent::alloc(1, true)?;
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
        };
        this.write_mmio_resp_addr();
        this.reset()?;
        this.init_queues()?;
        Ok(this)
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

        // The AENQ must exist even though we never enable any event group.
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
