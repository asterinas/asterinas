// SPDX-License-Identifier: MPL-2.0

//! Common configuration, split virtqueues, and workers for vhost backends.
//!
//! [`VhostWorker`](worker::VhostWorker) manages owner-bound execution and event
//! registration. Device callbacks borrow a locked [`VhostDevice`](device::VhostDevice)
//! to process requests; configuration changes and pause wait for that batch.
//! Backends retain protocol policy and stop the common worker on session close.

pub(super) mod device;
pub(super) mod memory;
pub(super) mod virtqueue;
pub(super) mod worker;
