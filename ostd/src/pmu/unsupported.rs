// SPDX-License-Identifier: MPL-2.0

//! Fallback PMU backend for architectures without hardware counter support.

use super::{EventConfig, PmuCaps, PmuError, Result};

pub(super) const HAS_BACKEND: bool = false;

/// A placeholder counter token on unsupported architectures.
#[derive(Debug)]
pub(super) enum Token {}

#[derive(Debug)]
pub(super) enum Caps {}

impl Caps {
    pub(super) fn summary(&self) -> PmuCaps {
        match *self {}
    }
}

pub(super) fn probe() -> Result<Caps> {
    Err(PmuError::Unsupported)
}

pub(super) fn configure(
    _config: EventConfig,
    _caps: &Caps,
    _used: &[Option<usize>],
) -> Result<(Token, u64)> {
    Err(PmuError::Unsupported)
}

impl Token {
    pub(super) fn index(&self) -> usize {
        match *self {}
    }

    pub(super) fn width_bits(&self) -> u8 {
        match *self {}
    }

    pub(super) fn read(&self) -> u64 {
        match *self {}
    }

    pub(super) fn start(&self) -> u64 {
        match *self {}
    }

    pub(super) fn stop(&self) -> u64 {
        match *self {}
    }

    pub(super) fn release(&self) {
        match *self {}
    }
}
