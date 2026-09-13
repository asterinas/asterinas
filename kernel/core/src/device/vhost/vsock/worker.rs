// SPDX-License-Identifier: MPL-2.0

//! The owner-VMAR-bound worker for both persistent vsock queues.

use core::sync::atomic::Ordering;

use super::{
    Backend, NUM_QUEUES, RX_QUEUE, TX_QUEUE, VhostVsockShared,
    packet::{self, HEADER_LEN, MAX_PAYLOAD_LEN},
};
use crate::{
    device::vhost::common::virtqueue::VhostQueue,
    events::IoEvents,
    net::socket::vsock::{self, VMADDR_CID_HOST},
    prelude::*,
    process::signal::{Pollable, Poller},
    thread::Thread,
};

const WORK_BUDGET: usize = 64;

pub(super) fn run(shared: Arc<VhostVsockShared>) {
    if run_queues(&shared).is_ok() {
        return;
    }
    let cid = {
        let mut common = shared.common.lock();
        common.deactivate();
        for index in 0..NUM_QUEUES {
            if let Ok(queue) = common.queue_mut(index) {
                queue.signal_error();
            }
        }
        let mut pending = shared.backend.pending.lock();
        pending.is_active = false;
        pending.failed = true;
        pending.generation = pending.generation.wrapping_add(1);
        pending.discard();
        shared.backend.cid()
    };
    if cid != 0 {
        vsock::reset_vhost_orphaned_connections();
    }
}

fn run_queues(shared: &VhostVsockShared) -> Result<()> {
    let backend = &shared.backend;
    loop {
        let mut poller = Poller::new(None);
        backend
            .wake
            .poll(IoEvents::IN, Some(poller.as_handle_mut()));
        let has_wakeup = backend.wake.consume().is_some();
        let mut common = shared.common.lock();
        if shared.exiting.load(Ordering::Acquire) {
            return Ok(());
        }
        if backend.pending.lock().failed {
            return_errno_with_message!(Errno::ENOBUFS, "the vsock endpoint failed");
        }
        let cid = backend.cid();
        if !common.is_running() {
            drop(common);
            poller.wait()?;
            continue;
        }
        for index in 0..NUM_QUEUES {
            if let Some(kick) = common.kick_event(index) {
                kick.poll(IoEvents::IN, Some(poller.as_handle_mut()));
            }
        }
        let mut failed = false;
        for index in 0..NUM_QUEUES {
            let mut queue = common.queue_mut(index)?;
            queue.consume_kick();
            if queue.disable_kick_notifications().is_err() {
                queue.signal_error();
                failed = true;
            }
        }
        let mut received_any = false;
        let mut transmitted_any = false;
        if !failed {
            for _ in 0..WORK_BUDGET {
                let received = match receive_packet(&mut common.queue_mut(RX_QUEUE)?, backend) {
                    Ok(received) => received,
                    Err(_) => {
                        common.queue_mut(RX_QUEUE)?.signal_error();
                        failed = true;
                        false
                    }
                };
                received_any |= received;
                if failed {
                    break;
                }
                let transmitted = if backend.pending.lock().has_control_room() {
                    match transmit_packet(&mut common.queue_mut(TX_QUEUE)?, cid) {
                        Ok(transmitted) => transmitted,
                        Err(_) => {
                            common.queue_mut(TX_QUEUE)?.signal_error();
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
                let queue = common.queue_mut(index)?;
                if queue.notify().is_err() {
                    queue.signal_error();
                    failed = true;
                }
            }
        }
        let did_work = received_any || transmitted_any;
        let mut retry = false;
        for index in 0..NUM_QUEUES {
            let mut queue = common.queue_mut(index)?;
            match queue.enable_kick_notifications() {
                Ok(ready) => {
                    let pending = backend.pending.lock();
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
        // Guest accesses, protocol dispatch, and completion notifications all
        // finish before control can replace memory or deactivate the queues.
        drop(common);
        if (did_work || has_wakeup) && cid != 0 {
            vsock::notify_vhost_writable(cid);
        }
        if !failed && (did_work || retry) {
            Thread::yield_now();
        } else {
            // Queue faults are reported through err; keep the worker and
            // configuration so a subsequent kick or reconfiguration can retry.
            poller.wait()?;
        }
    }
}

fn transmit_packet(queue: &mut VhostQueue<'_>, cid: u32) -> Result<bool> {
    let Some(chain) = queue.try_pop()? else {
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

fn receive_packet(queue: &mut VhostQueue<'_>, backend: &Backend) -> Result<bool> {
    let Some((packet, offset)) = backend.pending.lock().front() else {
        return Ok(false);
    };
    let Some(chain) = queue.try_pop()? else {
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
    let written = writer.bytes_written() as u32;
    chain.complete(written)?;
    backend.pending.lock().complete_fragment(len);
    Ok(true)
}
