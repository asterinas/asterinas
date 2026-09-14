// SPDX-License-Identifier: MPL-2.0

pub(crate) mod block_device;
mod namespace;
mod stat;

pub(crate) const MAX_NS_NUM: usize = 1024;

#[derive(Debug)]
pub(crate) enum NvmeDeviceError {
    BlockDeviceRegisterFailed,
    CommandFailed,
    ControllerEnableTimeout,
    InvalidControllerConfig,
    MsixAllocationFailed,
    NoNamespace,
    QueueAllocationFailed,
    SubmissionQueueFull,
    DmaAllocationFailed,
}
