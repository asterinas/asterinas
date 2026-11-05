// SPDX-License-Identifier: GPL-1.0+
// Register definitions used by the Linux-derived RTL8029 implementation.

pub(super) const REALTEK_VENDOR_ID: u16 = 0x10ec;
pub(super) const RTL8029_DEVICE_ID: u16 = 0x8029;
pub(super) const IO_EXTENT: u32 = 0x20;
pub(super) const PAGE_BYTES: usize = 256;
pub(super) const TX_START: u8 = 0x40;
pub(super) const TX_PAGES: u8 = 12;
pub(super) const RX_START: u8 = TX_START + TX_PAGES;
pub(super) const RX_END: u8 = 0x80;
pub(super) const RX_QUEUE_LIMIT: usize = 64;
pub(super) const MIN_FRAME_LEN: usize = 60;
pub(super) const MAX_FRAME_LEN: usize = 1518;
pub(super) const MAX_TX_FRAME_LEN: usize = 1514;
pub(super) const POLL_LIMIT: usize = 100_000;

pub(super) const CR: usize = 0;
pub(super) const PSTART: usize = 1;
pub(super) const PSTOP: usize = 2;
pub(super) const BNRY: usize = 3;
pub(super) const TPSR: usize = 4;
pub(super) const TBCR0: usize = 5;
pub(super) const TBCR1: usize = 6;
pub(super) const ISR: usize = 7;
pub(super) const RSAR0: usize = 8;
pub(super) const RSAR1: usize = 9;
pub(super) const RBCR0: usize = 0x0a;
pub(super) const RBCR1: usize = 0x0b;
pub(super) const RCR: usize = 0x0c;
pub(super) const TCR: usize = 0x0d;
pub(super) const DCR: usize = 0x0e;
pub(super) const IMR: usize = 0x0f;
pub(super) const RESET: usize = 0x1f;
// Page-one aliases.
pub(super) const PAR0: usize = 1;
pub(super) const CURR: usize = 7;
pub(super) const MAR0: usize = 8;

pub(super) const CMD_STOP: u8 = 0x21;
pub(super) const CMD_START: u8 = 0x22;
pub(super) const CMD_PAGE1_STOP: u8 = 0x61;
pub(super) const CMD_PAGE1_START: u8 = 0x62;
pub(super) const CMD_READ: u8 = 0x0a;
pub(super) const CMD_WRITE: u8 = 0x12;
pub(super) const CMD_TRANSMIT: u8 = 0x26;
pub(super) const DCR_WORD: u8 = 0x49;
pub(super) const RCR_MONITOR: u8 = 0x20;
pub(super) const RCR_BROADCAST: u8 = 4;
pub(super) const TCR_LOOPBACK: u8 = 2;
pub(super) const IRQ_MASK: u8 = 0x3f;
pub(super) const RX_IRQ: u8 = 0x05;
pub(super) const TX_IRQ: u8 = 0x0a;
pub(super) const OVER: u8 = 0x10;
pub(super) const COUNTERS: u8 = 0x20;
pub(super) const RDC: u8 = 0x40;
pub(super) const RESET_DONE: u8 = 0x80;
pub(super) const ACK_ALL: u8 = 0xff;

pub(super) enum InitializeMode {
    Stopped,
    Running,
}
