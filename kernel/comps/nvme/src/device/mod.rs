// SPDX-License-Identifier: MPL-2.0

pub mod block_device;
mod namespace;
mod stat;

use self::stat::NvmeStats;

pub(crate) const MAX_NS_NUM: usize = 1024;

#[derive(Debug)]
#[expect(dead_code)]
pub(crate) enum NvmeDeviceError {
    CommandFailed,
    MsixAllocationFailed,
    NoNamespace,
    QueuesAmountDoNotMatch,
}

pub(crate) use self::namespace::NvmeNamespace;
