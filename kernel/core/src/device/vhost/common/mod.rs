// SPDX-License-Identifier: MPL-2.0

//! Common configuration, split virtqueues, and workers for vhost backends.
//!
//! A vhost device processes queues in an owning userspace process's address space.
//! Each open file pairs [`VhostFileCommon`](device::VhostFileCommon),
//! which controls configuration and worker lifetime,
//! with [`VhostSharedState`](device::VhostSharedState),
//! which holds the guest memory regions, queues, and worker notifications.
//! The backend serializes ioctls and close with a mutex around common state;
//! the worker accesses shared state independently.
//!
//! The shared runtime mutex serializes queue processing with reconfiguration.
//! [`VhostMemorySpace`](memory::VhostMemorySpace) translates descriptor guest
//! addresses into owner addresses; vring addresses already refer to owner memory.
//! A VMAR reference does not pin mappings, so memory accesses remain fallible.
//! A [`VhostDescriptorChain`](virtqueue::VhostDescriptorChain) provides request
//! readers, response writers, and completion publication to the guest's used ring.
//!
//! To implement a backend:
//! 1. On open, create common state with the feature masks and queue size limit,
//!    and shared state with the backend's queue count.
//! 2. Handle `SET_OWNER` in the ioctl context: capture the current VMAR and call
//!    [`set_owner`](device::VhostFileCommon::set_owner) with shared state and a
//!    processing closure. The worker runs in that VMAR.
//! 3. Forward common ioctls to [`handle_ioctl`](device::VhostFileCommon::handle_ioctl)
//!    with the same shared state, and enable queues when the backend starts.
//! 4. In each callback, lock the runtime, check `is_running`, and borrow memory
//!    and queues together. Suppress kicks while draining; restore and recheck
//!    before returning `Idle`. Use `Pending` to continue after a bounded batch.
//! 5. On file close, call [`reset_owner`](device::VhostFileCommon::reset_owner)
//!    with shared state to stop and join before releasing owner resources.
//!    This is required even if the callback retains the backend.
//!
//! The executable toy backend in `common/tests/echo.rs` demonstrates this flow:
//! `EchoDevice::open` creates an `EchoFile`, whose worker copies each
//! four-byte request into a four-byte response. Its queue operation is:
//!
//! ```ignore
//! let Some(chain) = queue.try_pop(memory, features)? else {
//!     return Ok(false);
//! };
//! if chain.readable_len() != 4 || chain.writable_len() != 4 {
//!     return_errno_with_message!(Errno::EINVAL, "invalid echo request size");
//! }
//! let mut data = [0; 4];
//! chain.reader().read_exact(&mut data)?;
//! chain.writer().write_all(&data)?;
//! chain.complete(4)?;
//! queue.notify(memory)?;
//! ```
//!
//! Backends handle queue errors and retain device protocol, buffering, and
//! failure policy. They release the runtime mutex before calling socket code.
//! Socket producers can enqueue host packets and notify `worker_pollee` without
//! acquiring either sleeping mutex. Producers must update pending state before
//! notifying; the worker registers notifications before checking that state.

pub(super) mod device;
pub(super) mod memory;
pub(super) mod virtqueue;
pub(super) mod worker;

#[cfg(ktest)]
mod tests;
