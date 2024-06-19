// SPDX-License-Identifier: MPL-2.0

//! In the setup, VA - SETUP32_LMA == FileOffset - LEGACY_SETUP_SEC_SIZE.
//! And the addresses are specified in the ELF file.
//!
//! This module centralizes the conversion between VA and FileOffset.

use std::{
    cmp::PartialOrd,
    convert::From,
    ops::{Add, Sub},
};

// We chose the legacy setup sections to be 7 so that the setup header
// is page-aligned and the legacy setup section size would be 0x1000.
pub const LEGACY_SETUP_SECS: usize = 7;
pub const LEGACY_SETUP_SEC_SIZE: usize = 0x200 * (LEGACY_SETUP_SECS + 1);

pub const SETUP32_LMA: usize = 0x100000;

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct SetupVA {
    addr: usize,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Copy)]
pub struct SetupFileOffset {
    offset: usize,
}

impl From<usize> for SetupVA {
    fn from(addr: usize) -> Self {
        Self { addr }
    }
}

impl From<SetupVA> for usize {
    fn from(va: SetupVA) -> Self {
        va.addr
    }
}

impl Sub for SetupVA {
    type Output = usize;

    fn sub(self, rhs: Self) -> Self::Output {
        self.addr - rhs.addr
    }
}

impl Add<usize> for SetupVA {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self {
            addr: self.addr + rhs,
        }
    }
}

impl From<usize> for SetupFileOffset {
    fn from(offset: usize) -> Self {
        Self { offset }
    }
}

impl From<SetupFileOffset> for usize {
    fn from(offset: SetupFileOffset) -> Self {
        offset.offset
    }
}

impl Sub for SetupFileOffset {
    type Output = usize;

    fn sub(self, rhs: Self) -> Self::Output {
        self.offset - rhs.offset
    }
}

impl Add<usize> for SetupFileOffset {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self {
            offset: self.offset + rhs,
        }
    }
}

impl From<SetupVA> for SetupFileOffset {
    fn from(va: SetupVA) -> Self {
        Self {
            offset: va.addr + LEGACY_SETUP_SEC_SIZE - SETUP32_LMA,
        }
    }
}

impl From<SetupFileOffset> for SetupVA {
    fn from(offset: SetupFileOffset) -> Self {
        Self {
            addr: offset.offset + SETUP32_LMA - LEGACY_SETUP_SEC_SIZE,
        }
    }
}
