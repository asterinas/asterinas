// SPDX-License-Identifier: MPL-2.0

//! Common configuration, split virtqueues, and workers for vhost backends.
//!
//! [`VhostSession`](device::VhostSession) owns configuration and worker lifecycle;
//! the backend places it under a mutex to serialize control operations.
//! Callbacks use [`VhostSharedData`](device::VhostSharedData) to lock the current
//! memory and queues for a batch, then unlock before further backend work.
//! Backends retain protocol policy and stop the session's worker on close.

pub(super) mod device;
pub(super) mod memory;
pub(super) mod virtqueue;
pub(super) mod worker;
