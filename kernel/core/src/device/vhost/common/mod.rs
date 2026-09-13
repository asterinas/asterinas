// SPDX-License-Identifier: MPL-2.0

//! Common configuration, memory access, and split virtqueues for vhost backends.
//!
//! Device backends own protocol handling and worker lifecycles. They share a
//! [`VhostDevice`](device::VhostDevice) under a sleeping mutex and bind workers
//! to its owner VMAR. Queue handles and descriptor chains borrow the locked
//! device; reconfiguration and pause wait for current requests to finish.
//! Backends wake workers after configuration changes and join them on close.

pub(super) mod device;
pub(super) mod memory;
pub(super) mod virtqueue;
