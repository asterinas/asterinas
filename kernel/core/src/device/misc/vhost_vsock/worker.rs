// SPDX-License-Identifier: MPL-2.0

//! The owner-VMAR-bound worker for both vsock queues.

use super::{
    Backend, NUM_QUEUES, RX_QUEUE, TX_QUEUE,
    packet::{self, HEADER_LEN, MAX_PAYLOAD_LEN},
};
use crate::{
    device::misc::vhost::{VhostRuntime, VhostVirtQueue},
    events::{IoEvents, KernelEventFile},
    net::socket::vsock::{self, VMADDR_CID_HOST},
    prelude::*,
    process::signal::{Pollable, Poller},
    thread::Thread,
};

const WORK_BUDGET: usize = 64;

pub(super) fn run(
    mut runtime: VhostRuntime<NUM_QUEUES>,
    backend: Arc<Backend>,
    kicks: [Arc<KernelEventFile>; NUM_QUEUES],
) {
    let result = run_queues(&mut runtime, &backend, &kicks).and_then(|()| {
        // Stop may race with a batch after add_used. Complete notification
        // before joining, even when that batch exits at a stop check.
        for index in 0..NUM_QUEUES {
            runtime.queue_mut(index)?.notify()?;
        }
        Ok(())
    });
    if result.is_err() || backend.pending.lock().failed {
        for index in 0..NUM_QUEUES {
            if let Ok(queue) = runtime.queue_mut(index) {
                queue.signal_error();
            }
        }
        {
            let mut pending = backend.pending.lock();
            pending.is_active = false;
            pending.is_running = false;
            pending.failed = true;
            pending.generation = pending.generation.wrapping_add(1);
            pending.discard();
        }
        vsock::reset_vhost_connections(backend.cid);
    }
}

fn run_queues(
    runtime: &mut VhostRuntime<NUM_QUEUES>,
    backend: &Backend,
    kicks: &[Arc<KernelEventFile>; NUM_QUEUES],
) -> Result<()> {
    loop {
        let mut poller = Poller::new(None);
        for kick in kicks {
            kick.poll(IoEvents::IN, Some(poller.as_handle_mut()));
        }
        backend
            .wake
            .poll(IoEvents::IN, Some(poller.as_handle_mut()));
        let has_wakeup = backend.wake.consume().is_some();
        if !backend.is_running() {
            return Ok(());
        }
        if has_wakeup {
            vsock::notify_vhost_writable(backend.cid);
        }
        for index in 0..NUM_QUEUES {
            let queue = runtime.queue_mut(index)?;
            queue.consume_kick();
            queue.disable_kick_notifications()?;
        }

        let mut did_work = false;
        for _ in 0..WORK_BUDGET {
            if !backend.is_running() {
                return Ok(());
            }
            let received = receive_packet(runtime.queue_mut(RX_QUEUE)?, backend)?;
            // Retain space for responses before accepting more guest requests.
            let transmitted = if backend.pending.lock().has_control_room() {
                transmit_packet(runtime.queue_mut(TX_QUEUE)?, backend.cid)?
            } else {
                false
            };
            if !received && !transmitted {
                break;
            }
            did_work = true;
        }
        if did_work {
            runtime.queue_mut(RX_QUEUE)?.notify()?;
            runtime.queue_mut(TX_QUEUE)?.notify()?;
            vsock::notify_vhost_writable(backend.cid);
            Thread::yield_now();
            continue;
        }

        let rx_ready = runtime.queue_mut(RX_QUEUE)?.enable_kick_notifications()?;
        let tx_ready = runtime.queue_mut(TX_QUEUE)?.enable_kick_notifications()?;
        let pending = backend.pending.lock();
        if !pending.is_running {
            return Ok(());
        }
        let retry =
            (rx_ready && pending.front().is_some()) || (tx_ready && pending.has_control_room());
        drop(pending);
        if !retry {
            poller.wait()?;
        }
    }
}

fn transmit_packet(queue: &mut VhostVirtQueue, cid: u32) -> Result<bool> {
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
    queue.add_used(&chain, 0)?;
    Ok(true)
}

fn receive_packet(queue: &mut VhostVirtQueue, backend: &Backend) -> Result<bool> {
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
    queue.add_used(&chain, writer.bytes_written() as u32)?;
    backend.pending.lock().complete_fragment(len);
    Ok(true)
}
