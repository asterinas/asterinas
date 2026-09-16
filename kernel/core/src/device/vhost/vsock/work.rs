// SPDX-License-Identifier: MPL-2.0

//! Vsock packet processing executed by the common vhost worker.

use aster_virtio::Feature;

use super::{
    Backend, NUM_QUEUES, RX_QUEUE, TX_QUEUE,
    packet::{self, HEADER_LEN, MAX_PAYLOAD_LEN},
};
use crate::{
    device::vhost::common::{
        device::{VhostRuntimeState, VhostSharedState},
        memory::VhostMemorySpace,
        virtqueue::VhostVirtQueue,
        worker::VhostWorkStatus,
    },
    events::IoEvents,
    net::socket::vsock::{self, VMADDR_CID_HOST},
    prelude::*,
};

const WORK_BUDGET: usize = 64;

impl Backend {
    pub(super) fn process(&self, shared: &VhostSharedState<NUM_QUEUES>) -> VhostWorkStatus {
        self.reset_failed_endpoint();
        let mut common = shared.runtime().lock();
        let status = if common.is_running() {
            self.process_queues(&mut common)
        } else {
            VhostWorkStatus::Idle
        };
        drop(common);
        let cid = self.cid();
        if cid != 0 {
            vsock::notify_vhost_writable(cid);
        }
        status
    }

    // Ordinary queue faults only end a batch. Losing a host control packet is
    // different: the socket operation cannot always be retried. Its producer
    // requests this deferred endpoint reset without sleeping under socket locks.
    fn reset_failed_endpoint(&self) {
        let cid = {
            let mut common = self.shared.runtime().lock();
            let mut pending = self.pending.lock();
            if !pending.needs_reset {
                return;
            }
            pending.is_active = false;
            pending.generation = pending.generation.wrapping_add(1);
            pending.discard();
            drop(pending);
            common.disable_queues();
            if let Ok((_, queues)) = common.memory_and_queues_mut() {
                for queue in queues {
                    queue.signal_error();
                }
            }
            self.cid()
        };
        if cid != 0 {
            vsock::reset_vhost_orphaned_connections();
        }
        self.pending.lock().needs_reset = false;
        self.shared.worker_pollee().notify(IoEvents::IN);
    }

    fn process_queues(&self, common: &mut VhostRuntimeState<NUM_QUEUES>) -> VhostWorkStatus {
        if !self.pending.lock().is_active {
            return VhostWorkStatus::Idle;
        }
        let cid = self.cid();
        let features = Feature::from_bits_truncate(common.negotiated_features());
        let (memory, queues) = common
            .memory_and_queues_mut()
            .expect("enabled queues have owner memory");
        let mut failed = false;
        for queue in queues.iter_mut() {
            if queue.disable_kick_notifications(memory).is_err() {
                queue.signal_error();
                failed = true;
            }
        }
        let mut received_any = false;
        let mut transmitted_any = false;
        if !failed {
            for _ in 0..WORK_BUDGET {
                let received = match receive_packet(&mut queues[RX_QUEUE], memory, features, self) {
                    Ok(received) => received,
                    Err(_) => {
                        queues[RX_QUEUE].signal_error();
                        failed = true;
                        false
                    }
                };
                received_any |= received;
                if failed {
                    break;
                }
                let transmitted = if self.pending.lock().has_control_room() {
                    match transmit_packet(&mut queues[TX_QUEUE], memory, features, cid) {
                        Ok(transmitted) => transmitted,
                        Err(_) => {
                            queues[TX_QUEUE].signal_error();
                            failed = true;
                            false
                        }
                    }
                } else {
                    false
                };
                transmitted_any |= transmitted;
                if failed || (!received && !transmitted) {
                    break;
                }
            }
        }
        for (index, completed) in [(RX_QUEUE, received_any), (TX_QUEUE, transmitted_any)] {
            if completed {
                let queue = &queues[index];
                if queue.notify(memory).is_err() {
                    queue.signal_error();
                    failed = true;
                }
            }
        }
        let did_work = received_any || transmitted_any;
        let mut retry = false;
        for (index, queue) in queues.iter_mut().enumerate() {
            match queue.enable_kick_notifications(memory) {
                Ok(ready) => {
                    let pending = self.pending.lock();
                    retry |= ready
                        && if index == RX_QUEUE {
                            pending.front().is_some()
                        } else {
                            pending.has_control_room()
                        };
                }
                Err(_) => {
                    queue.signal_error();
                    failed = true;
                }
            }
        }
        if !failed && (did_work || retry) {
            VhostWorkStatus::Pending
        } else {
            VhostWorkStatus::Idle
        }
    }
}

fn transmit_packet(
    queue: &mut VhostVirtQueue,
    memory: &VhostMemorySpace,
    features: Feature,
    cid: u32,
) -> Result<bool> {
    let Some(chain) = queue.try_pop(memory, features)? else {
        return Ok(false);
    };
    if chain.writable_len() != 0 || chain.readable_len() < HEADER_LEN {
        return_errno_with_message!(
            Errno::EINVAL,
            "invalid vsock transmit descriptor direction or size"
        );
    }
    let mut bytes = [0; HEADER_LEN];
    let mut reader = chain.reader();
    reader.read_exact(&mut bytes)?;
    let header = packet::decode_header(&bytes);
    let len = header.len as usize;
    if header.src_cid != u64::from(cid)
        || header.dst_cid != u64::from(VMADDR_CID_HOST)
        || len > MAX_PAYLOAD_LEN
        || len > reader.remaining()
    {
        return_errno_with_message!(Errno::EINVAL, "invalid vsock transmit packet");
    }
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload)?;
    vsock::handle_vhost_packet(header, &payload)?;
    // TX descriptors are only read by the backend.
    chain.complete(0)?;
    Ok(true)
}

fn receive_packet(
    queue: &mut VhostVirtQueue,
    memory: &VhostMemorySpace,
    features: Feature,
    backend: &Backend,
) -> Result<bool> {
    let Some((packet, offset)) = backend.pending.lock().front() else {
        return Ok(false);
    };
    let Some(chain) = queue.try_pop(memory, features)? else {
        return Ok(false);
    };
    if chain.readable_len() != 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "the vsock receive chain is not writable-only"
        );
    }
    let header = packet.header_for_fragment(offset, chain.writable_len())?;
    let len = header.len as usize;
    let wire_header = packet::encode_header(header);
    let mut writer = chain.writer();
    writer.write_all(wire_header.as_bytes())?;
    writer.write_all(&packet.payload[offset..offset + len])?;
    let written = (wire_header.as_bytes().len() + len) as u32;
    chain.complete(written)?;
    backend.pending.lock().complete_fragment(len);
    Ok(true)
}
