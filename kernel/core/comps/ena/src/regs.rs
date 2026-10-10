// SPDX-License-Identifier: MPL-2.0

//! ENA register map (BAR0) and bit definitions.
//!
//! Reference: `drivers/net/ethernet/amazon/ena/ena_regs_defs.h` in Linux,
//! which mirrors the ENA specification.

#![expect(dead_code)]

pub(crate) const VERSION: usize = 0x0;
pub(crate) const CONTROLLER_VERSION: usize = 0x4;
pub(crate) const CAPS: usize = 0x8;
pub(crate) const CAPS_EXT: usize = 0xc;
pub(crate) const AQ_BASE_LO: usize = 0x10;
pub(crate) const AQ_BASE_HI: usize = 0x14;
pub(crate) const AQ_CAPS: usize = 0x18;
pub(crate) const ACQ_BASE_LO: usize = 0x20;
pub(crate) const ACQ_BASE_HI: usize = 0x24;
pub(crate) const ACQ_CAPS: usize = 0x28;
pub(crate) const AQ_DB: usize = 0x2c;
pub(crate) const ACQ_TAIL: usize = 0x30;
pub(crate) const AENQ_CAPS: usize = 0x34;
pub(crate) const AENQ_BASE_LO: usize = 0x38;
pub(crate) const AENQ_BASE_HI: usize = 0x3c;
pub(crate) const AENQ_HEAD_DB: usize = 0x40;
pub(crate) const AENQ_TAIL: usize = 0x44;
pub(crate) const INTR_MASK: usize = 0x4c;
pub(crate) const DEV_CTL: usize = 0x54;
pub(crate) const DEV_STS: usize = 0x58;
pub(crate) const MMIO_REG_READ: usize = 0x5c;
pub(crate) const MMIO_RESP_LO: usize = 0x60;
pub(crate) const MMIO_RESP_HI: usize = 0x64;

/// BAR0 must be at least this large to hold the fixed registers.
pub(crate) const BAR0_MIN_SIZE: u64 = 0x100;

pub(crate) const CAPS_RESET_TIMEOUT_SHIFT: u32 = 1;
pub(crate) const CAPS_RESET_TIMEOUT_MASK: u32 = 0x3e;
pub(crate) const CAPS_DMA_ADDR_WIDTH_SHIFT: u32 = 8;
pub(crate) const CAPS_DMA_ADDR_WIDTH_MASK: u32 = 0xff00;
pub(crate) const CAPS_ADMIN_CMD_TO_SHIFT: u32 = 16;
pub(crate) const CAPS_ADMIN_CMD_TO_MASK: u32 = 0xf0000;

pub(crate) const AQ_CAPS_DEPTH_MASK: u32 = 0xffff;
pub(crate) const AQ_CAPS_ENTRY_SIZE_SHIFT: u32 = 16;

pub(crate) const DEV_CTL_DEV_RESET: u32 = 0x1;
pub(crate) const DEV_CTL_RESET_REASON_SHIFT: u32 = 28;
pub(crate) const RESET_REASON_NORMAL: u32 = 0;

pub(crate) const DEV_STS_READY: u32 = 0x1;
pub(crate) const DEV_STS_RESET_IN_PROGRESS: u32 = 0x8;
pub(crate) const DEV_STS_FATAL_ERROR: u32 = 0x20;

pub(crate) const MMIO_REG_READ_REQ_ID_MASK: u32 = 0xffff;
pub(crate) const MMIO_REG_READ_REG_OFF_SHIFT: u32 = 16;

/// Bit 0 of `INTR_MASK` masks the admin/AENQ interrupt (we poll the admin queue).
pub(crate) const ADMIN_INTR_MASK: u32 = 0x1;

/// Value returned by the readless MMIO path on timeout, like Linux's `ENA_MMIO_READ_TIMEOUT`.
pub(crate) const MMIO_READ_TIMEOUT: u32 = 0xffff_ffff;
