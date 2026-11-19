// SPDX-License-Identifier: MPL-2.0
//! Specific implementation of KVM (architecture-specific components)

pub(super) mod vcpu;
pub(super) mod vm;
pub(super) mod ioctl;
pub(super) mod mmio;
pub mod insn;
pub mod isa;
pub mod onereg;
pub mod sbi;

use crate::prelude::*;
use super::KvmRegSize;
use onereg::{KVM_REG_SIZE_U32, KVM_REG_SIZE_U64, KVM_REG_SIZE_MASK};

#[derive(PartialEq)]
enum KvmExitReturn {
    ExitToUserSpace,
	ContinueRun,
}

impl From<usize> for KvmExitReturn {
    fn from(data: usize) -> Self {
        match data {
            0 => Self::ExitToUserSpace,
            _ => Self::ContinueRun,
        }
    }
}

impl KvmRegSize {
    pub fn from_id(id: usize) -> Result<Self> {
        match id & KVM_REG_SIZE_MASK {
            KVM_REG_SIZE_U32 => Ok(Self::U32),
            KVM_REG_SIZE_U64 => Ok(Self::U64),
            _ => Err(Error::new(Errno::EINVAL)),
        }
    }
}
