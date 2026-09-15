// SPDX-License-Identifier: MPL-2.0

//! Common configuration, split virtqueues, and workers for vhost backends.
//!
//! [`VhostDeviceSession`](device::VhostDeviceSession) owns the worker and serializes control and
//! data access. Callbacks borrow its locked [`VhostDeviceData`](device::VhostDeviceData)
//! to process requests; configuration changes and pause wait for that batch.
//! Backends retain protocol policy and stop the device's worker on session close.

pub(super) mod device;
pub(super) mod memory;
pub(super) mod virtqueue;
pub(super) mod worker;
