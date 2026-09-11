// SPDX-License-Identifier: MPL-2.0

//! Common configuration, memory access, and split virtqueues for vhost backends.
//!
//! Device backends own protocol handling and worker lifecycles. They configure
//! [`VhostDeviceState`](device::VhostDeviceState), build a [`VhostRuntime`](device::VhostRuntime) in the owner context, and bind
//! workers to its VMAR. Workers must be stopped, woken, and joined before
//! reconfiguration, owner reset, or close.

pub(super) mod device;
pub(super) mod memory;
pub(super) mod virtqueue;
