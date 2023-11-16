//! In the trojan, VA - SETUP32_LMA == FileOffset - LEGACY_SETUP_SEC_SIZE.
//! And the addresses are specified in the ELF file.

use std::{cmp::PartialOrd, convert::From, ops::Sub};

// We chose the legacy setup sections to be 7 so that the setup header
// is page-aligned and the legacy setup section size would be 0x1000.
pub const LEGACY_SETUP_SECS: usize = 7;
pub const LEGACY_SETUP_SEC_SIZE: usize = 0x200 * (LEGACY_SETUP_SECS + 1);

pub const SETUP32_LMA: usize = 0x100000;

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct TrojanVA {
    addr: usize,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct TrojanFileOffset {
    offset: usize,
}

impl From<usize> for TrojanVA {
    fn from(addr: usize) -> Self {
        Self { addr }
    }
}

impl From<TrojanVA> for usize {
    fn from(va: TrojanVA) -> Self {
        va.addr
    }
}

impl Sub for TrojanVA {
    type Output = usize;

    fn sub(self, rhs: Self) -> Self::Output {
        self.addr - rhs.addr
    }
}

impl From<usize> for TrojanFileOffset {
    fn from(offset: usize) -> Self {
        Self { offset }
    }
}

impl From<TrojanFileOffset> for usize {
    fn from(offset: TrojanFileOffset) -> Self {
        offset.offset
    }
}

impl Sub for TrojanFileOffset {
    type Output = usize;

    fn sub(self, rhs: Self) -> Self::Output {
        self.offset - rhs.offset
    }
}

impl From<TrojanVA> for TrojanFileOffset {
    fn from(va: TrojanVA) -> Self {
        Self {
            offset: va.addr + LEGACY_SETUP_SEC_SIZE - SETUP32_LMA,
        }
    }
}

impl From<TrojanFileOffset> for TrojanVA {
    fn from(offset: TrojanFileOffset) -> Self {
        Self {
            addr: offset.offset + SETUP32_LMA - LEGACY_SETUP_SEC_SIZE,
        }
    }
}
