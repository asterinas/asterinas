// SPDX-License-Identifier: MPL-2.0

//! A four-byte echo backend exercising a configured runtime and worker teardown.

use core::{sync::atomic, time::Duration};

use super::*;
use crate::{
    events::IoEvents,
    process::signal::{Pollable, Poller},
    thread::Thread,
};

struct EchoDevice {
    common: VhostDeviceState<1>,
    worker: Option<Arc<Thread>>,
    stop: Arc<KernelEventFile>,
    idle: Arc<KernelEventFile>,
}

impl EchoDevice {
    fn new() -> Result<Self> {
        Ok(Self {
            common: VhostDeviceState::new(VhostDeviceConfig {
                device_features: VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC,
                backend_features: 0,
                max_queue_size: 256,
            }),
            worker: None,
            stop: KernelEventFile::from_file(&EventFile::new(0, EventFileFlags::empty()))?,
            idle: event(),
        })
    }

    fn start(&mut self, mut runtime: VhostRuntime<1>) -> Result<()> {
        if self.worker.is_some() {
            return_errno_with_message!(Errno::EBUSY, "echo worker is already started");
        }
        let kick = runtime.queue_mut(0)?.kick_event().ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "echo worker needs a kick eventfd")
        })?;
        self.stop.consume();
        let stop = self.stop.clone();
        let idle = self.idle.clone();
        let vmar = runtime.vmar().clone();
        self.worker = Some(
            ThreadOptions::new(move || {
                // Reconfiguration joins this worker before invalidating its runtime.
                let queue = runtime.queue_mut(0).unwrap();
                if Self::run(queue, &kick, &stop, &idle).is_err() {
                    queue.signal_error();
                }
            })
            .vmar(vmar)
            .spawn(),
        );
        Ok(())
    }

    fn run(
        queue: &mut VhostVirtQueue,
        kick: &KernelEventFile,
        stop: &KernelEventFile,
        idle: &KernelEventFile,
    ) -> Result<()> {
        loop {
            let mut poller = Poller::new(None);
            kick.poll(IoEvents::IN, Some(poller.as_handle_mut()));
            if !stop
                .poll(IoEvents::IN, Some(poller.as_handle_mut()))
                .is_empty()
            {
                return Ok(());
            }
            queue.consume_kick();
            queue.disable_kick_notifications()?;
            if let Some(chain) = queue.try_pop()? {
                if chain.readable_len() != 4 || chain.writable_len() != 4 {
                    return_errno_with_message!(Errno::EINVAL, "invalid echo request size");
                }
                let mut data = [0u8; 4];
                chain.reader().read_exact(&mut data)?;
                let mut writer = chain.writer();
                writer.write_all(&data)?;
                queue.add_used(&chain, writer.bytes_written() as u32)?;
                queue.notify()?;
                Thread::yield_now();
            } else if !queue.enable_kick_notifications()? {
                idle.signal();
                poller.wait()?;
            }
        }
    }

    fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.signal();
            worker.join();
        }
    }
}

impl Drop for EchoDevice {
    fn drop(&mut self) {
        self.stop();
    }
}

fn wait_event(event: &KernelEventFile) {
    let mut poller = Poller::new(Some(&Duration::from_secs(5)));
    loop {
        event.poll(IoEvents::IN, Some(poller.as_handle_mut()));
        if event.consume().is_some() {
            return;
        }
        poller.wait().unwrap();
    }
}

#[ktest]
fn vhost_echo_worker_copies_notifies_and_stops_before_reset() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    with_owner_memory(|memory, vmar| {
        let mut echo = EchoDevice::new().unwrap();
        // Control-plane ioctls require a POSIX caller. This kernel-thread fixture
        // supplies the configured state and runtime directly.
        echo.common.owner_vmar = Some(vmar.clone());
        echo.common.memory_regions = vec![VhostMemoryRegion {
            guest_phys_addr: GUEST_ADDR,
            memory_size: 0x2000,
            host_virt_addr: GUEST_UVA as u64,
            flags_padding: 0,
        }];
        echo.common.negotiated_features = VIRTIO_F_VERSION_1 | VIRTIO_RING_F_INDIRECT_DESC;
        echo.common.queues[0] = queue_state();
        assert!(echo.common.is_fully_configured());
        let kick = event();
        let call = event();
        let err = event();
        echo.common.queues[0].kick = Some(kick.clone());
        echo.common.queues[0].call = Some(call.clone());
        echo.common.queues[0].err = Some(err.clone());
        echo.start(runtime(&echo.common, vmar.clone(), memory.clone()))
            .unwrap();
        wait_event(&echo.idle);

        memory.store(
            DESC_ADDR,
            &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT, 1),
        );
        memory.store(
            DESC_ADDR + size_of::<Descriptor>(),
            &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::WRITE, 0),
        );
        memory.write_owner_bytes(GUEST_UVA, b"echo").unwrap();
        memory.store(AVAIL_ADDR + AvailRing::entry_offset(0).unwrap(), &0u16);
        atomic::fence(Ordering::Release);
        memory
            .write_owner_val(AVAIL_ADDR + AvailRing::IDX_OFFSET, &1u16)
            .unwrap();
        kick.signal();
        wait_event(&call);

        let mut response = [0; 4];
        memory
            .read_owner_bytes(GUEST_UVA + 4, &mut response)
            .unwrap();
        assert_eq!(&response, b"echo");
        assert_eq!(memory.load::<UsedRing>(USED_ADDR).idx(), 1);
        let used = memory.load::<UsedElem>(USED_ADDR + UsedRing::entry_offset(0).unwrap());
        assert_eq!(used.id(), 0);
        assert_eq!(used.len(), 4);
        assert_eq!(err.consume(), None);
        wait_event(&echo.idle);

        // A backend must join before returning the queue base or resetting ownership.
        echo.stop();
        assert!(echo.worker.is_none());
        assert_eq!(echo.common.queue_base(0).unwrap(), 1);
        echo.start(runtime(&echo.common, vmar.clone(), memory.clone()))
            .unwrap();
        wait_event(&echo.idle);
        echo.stop();
        echo.common.reset_owner_after_quiesce();
        assert!(echo.worker.is_none());
        assert!(!echo.common.is_owned());
        assert!(!echo.common.is_fully_configured());
        assert_eq!(err.consume(), None);
    });
}
