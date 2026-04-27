// SPDX-License-Identifier: MPL-2.0

//! Queues of a virtio-vsock device.

use alloc::{boxed::Box, collections::vec_deque::VecDeque, vec::Vec};

use aster_util::slot_vec::SlotVec;
use ostd::mm::{
    dma::{DmaStream, FromDevice},
    io::util::HasVmReaderWriter,
};

use crate::{
    device::{
        VirtioDeviceError,
        socket::{
            header::{VirtioVsockEvent, VirtioVsockEventId, VirtioVsockHdr},
            packet::{RxPacket, TxPacket},
        },
    },
    queue::VirtQueue,
    transport::VirtioTransport,
};

/// The transmit queue of a virtio-vsock device.
///
/// The transmit path has two stages:
/// - The inflight queue is the hardware virtqueue and contains packets already submitted to the
///   device.
/// - The pending queue is a software queue and contains packets still waiting for a free virtqueue
///   descriptor.
///
/// Packets move from the pending queue to the inflight queue after the device finishes
/// transmitting earlier packets and frees descriptor space.
///
/// To bound pending resource usage, a pending packet may carry a [`TxCompletion`]. When the packet
/// is promoted from the pending queue to the inflight queue, [`TxCompletion::on_pending_submit`]
/// runs so the upper layer can release the resources reserved for that pending transmission, such
/// as socket send-queue capacity for newly queued data.
pub struct TxQueue {
    queue: VirtQueue,
    inflight: Vec<Option<TxPacket>>,
    pending: VecDeque<PendingTx>,
}

impl TxQueue {
    pub(super) const QUEUE_INDEX: u16 = 1;
    const QUEUE_SIZE: u16 = 64;

    pub(super) fn new(transport: &mut dyn VirtioTransport) -> Result<Self, VirtioDeviceError> {
        let queue = VirtQueue::new(Self::QUEUE_INDEX, Self::QUEUE_SIZE, transport)?;

        let inflight = (0..Self::QUEUE_SIZE).map(|_| None).collect();
        let pending = VecDeque::new();

        Ok(Self {
            queue,
            inflight,
            pending,
        })
    }

    pub(super) fn free_processed_tx_buffers(&mut self) {
        while let Ok((token, _)) = self.queue.pop_used() {
            debug_assert!(self.inflight[token as usize].is_some());
            self.inflight[token as usize] = None;
        }

        while self.queue.available_desc() >= 1 {
            let Some(pending) = self.pending.pop_front() else {
                break;
            };
            let PendingTx { packet, completion } = pending;

            if let Some(completion) = completion {
                completion.on_pending_submit();
            }

            let token = self.queue.add_input_bufs(&[packet.inner()]).unwrap();

            debug_assert!(self.inflight[token as usize].is_none());
            self.inflight[token as usize] = Some(packet);
        }

        if self.queue.should_notify() {
            self.queue.notify();
        }
    }

    /// Tries to submit `packet` to the inflight queue immediately, or returns a guard for
    /// submitting the packet to the pending queue.
    pub fn try_send(&mut self, packet: TxPacket) -> Result<(), TxPendingGuard<'_>> {
        if !self.pending.is_empty() || self.queue.available_desc() == 0 {
            return Err(TxPendingGuard {
                queue: self,
                packet,
            });
        }

        let token = self.queue.add_input_bufs(&[packet.inner()]).unwrap();

        debug_assert!(self.inflight[token as usize].is_none());
        self.inflight[token as usize] = Some(packet);

        if self.queue.should_notify() {
            self.queue.notify();
        }

        Ok(())
    }
}

struct PendingTx {
    packet: TxPacket,
    completion: Option<Box<dyn TxCompletion>>,
}

/// A callback invoked when a pending packet is submitted to the inflight queue.
///
/// This is primarily used for resource accounting. For details, see the [`TxQueue`] documentation.
pub trait TxCompletion: Send + Sync {
    /// Runs when the associated packet moves from the pending queue to the inflight queue.
    ///
    /// This method is called while the transmit queue lock is held.
    fn on_pending_submit(self: Box<Self>);
}

/// A guard holding a packet returned from [`TxQueue::try_send`] for deferred submission.
pub struct TxPendingGuard<'a> {
    queue: &'a mut TxQueue,
    packet: TxPacket,
}

impl TxPendingGuard<'_> {
    /// Enqueues the packet in the pending queue with an optional completion callback.
    pub fn push_pending(self, completion: Option<Box<dyn TxCompletion>>) {
        self.queue.pending.push_back(PendingTx {
            packet: self.packet,
            completion,
        });
    }
}

/// The receive queue of a virtio-vsock device.
pub struct RxQueue {
    queue: VirtQueue,
    buffers: SlotVec<RxPacket>,
    pending: Option<RxPacket>,
}

impl RxQueue {
    pub(super) const QUEUE_INDEX: u16 = 0;
    const QUEUE_SIZE: u16 = 64;

    pub(super) fn new(transport: &mut dyn VirtioTransport) -> Result<Self, VirtioDeviceError> {
        let mut queue = VirtQueue::new(Self::QUEUE_INDEX, Self::QUEUE_SIZE, transport)?;

        let mut buffers = SlotVec::new();
        for index in 0..Self::QUEUE_SIZE {
            let buffer = RxPacket::new().map_err(VirtioDeviceError::ResourceAlloc)?;
            let token = queue.add_output_bufs(&[buffer.inner()]).unwrap();
            assert_eq!(token, index);
            assert_eq!(buffers.put(buffer) as u16, index);
        }

        if queue.should_notify() {
            queue.notify();
        }

        Ok(Self {
            queue,
            buffers,
            pending: None,
        })
    }

    /// Returns the next received packet, if any.
    ///
    /// Packets shorter than the mandatory header are discarded before they reach the caller.
    pub fn recv(&mut self) -> Option<RxPacket> {
        if self.pending.is_none() {
            self.pending = RxPacket::new().ok();
        }
        if self.pending.is_none() {
            ostd::warn!("allocating recv packet fails");
            // FIXME: We should find ways to address the error to prevent the receive queue from
            // getting stuck.
            return None;
        }

        let (token, len) = self
            .queue
            .pop_used_with_min_bytes(size_of::<VirtioVsockHdr>())
            .ok()?;
        let mut packet = self.buffers.remove(token as usize).unwrap();
        packet.set_payload_len(len as usize - size_of::<VirtioVsockHdr>());

        let new_packet = self.pending.take().unwrap();
        let new_token = self.queue.add_output_bufs(&[new_packet.inner()]).unwrap();
        debug_assert_eq!(new_token, token);
        self.buffers.put_at(new_token as usize, new_packet);

        if self.queue.should_notify() {
            self.queue.notify();
        }

        Some(packet)
    }
}

/// The event queue of a virtio-vsock device.
pub(super) struct EventQueue {
    queue: VirtQueue,
    buffer: DmaStream<FromDevice>,
}

impl EventQueue {
    pub(super) const QUEUE_INDEX: u16 = 2;
    const QUEUE_SIZE: u16 = 1;

    pub(super) fn new(transport: &mut dyn VirtioTransport) -> Result<Self, VirtioDeviceError> {
        let mut queue = VirtQueue::new(Self::QUEUE_INDEX, Self::QUEUE_SIZE, transport)?;

        let buffer = DmaStream::alloc_uninit(1, false).map_err(VirtioDeviceError::ResourceAlloc)?;
        let token = queue.add_output_bufs(&[&buffer]).unwrap();
        debug_assert_eq!(token, 0);

        if queue.should_notify() {
            queue.notify();
        }

        Ok(Self { queue, buffer })
    }

    /// Returns the next received event, if any.
    pub(super) fn recv(&mut self) -> Option<VirtioVsockEventId> {
        let (token, len) = self
            .queue
            .pop_used_with_min_bytes(size_of::<VirtioVsockEvent>())
            .ok()?;
        debug_assert_eq!(token, 0);
        if len as usize != size_of::<VirtioVsockEvent>() {
            ostd::warn!("unexpected event length {}, ignoring trailing garbage", len);
        }

        self.buffer
            .sync_from_device(0..size_of::<VirtioVsockEvent>())
            .unwrap();
        let event = self
            .buffer
            .reader()
            .unwrap()
            .read_val::<VirtioVsockEvent>()
            .unwrap();
        let event_id = VirtioVsockEventId::try_from(event.id).ok();

        let token = self.queue.add_output_bufs(&[&self.buffer]).unwrap();
        debug_assert_eq!(token, 0);

        if self.queue.should_notify() {
            self.queue.notify();
        }

        event_id
    }
}
