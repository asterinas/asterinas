// SPDX-License-Identifier: MPL-2.0

//! A four-byte echo backend using the common vhost worker.

use core::{
    sync::atomic::{self, AtomicBool},
    time::Duration,
};

use super::*;
use crate::{
    device::vhost::common::worker::{VhostWork, VhostWorkStatus, VhostWorker},
    events::IoEvents,
    process::signal::{Pollable, Poller},
};

struct EchoDevice {
    common: Arc<Mutex<VhostDevice<1>>>,
    worker: VhostWorker<1>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl EchoDevice {
    fn new(mut device: VhostDevice<1>) -> Self {
        device.deactivate();
        let vmar = device.owner_vmar().unwrap().clone();
        let common = Arc::new(Mutex::new(device));
        let mut worker = VhostWorker::new(common.clone());
        let idle = create_event();
        let completed = Arc::new(AtomicBool::new(false));
        worker.start(
            vmar,
            EchoWork {
                common: common.clone(),
                idle: idle.clone(),
                completed: completed.clone(),
            },
        );
        Self {
            common,
            worker,
            idle,
            completed,
        }
    }
}

struct EchoWork {
    common: Arc<Mutex<VhostDevice<1>>>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl VhostWork<1> for EchoWork {
    fn process(&mut self, device: &mut VhostDevice<1>) -> Result<VhostWorkStatus> {
        let mut queue = device.queue_mut(0)?;
        queue.disable_kick_notifications()?;
        if Self::copy_next(&mut queue)? {
            queue.notify()?;
            return Ok(VhostWorkStatus::Pending);
        }
        Ok(if queue.enable_kick_notifications()? {
            VhostWorkStatus::Pending
        } else {
            VhostWorkStatus::Idle
        })
    }

    fn after_process(&mut self, status: VhostWorkStatus) -> Result<()> {
        if status == VhostWorkStatus::Idle {
            self.idle.signal();
        }
        Ok(())
    }

    fn on_exit(self, result: Result<()>) {
        if result.is_err() {
            self.common.lock().queue_mut(0).unwrap().signal_error();
        }
        self.completed.store(result.is_ok(), Ordering::Release);
    }
}

impl EchoWork {
    fn copy_next(queue: &mut VhostQueue<'_>) -> Result<bool> {
        let Some(chain) = queue.try_pop()? else {
            return Ok(false);
        };
        if chain.readable_len() != 4 || chain.writable_len() != 4 {
            return_errno_with_message!(Errno::EINVAL, "invalid echo request size");
        }
        let mut data = [0u8; 4];
        chain.reader().read_exact(&mut data)?;
        chain.writer().write_all(&data)?;
        chain.complete(4)?;
        Ok(true)
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
fn vhost_echo_worker_preserves_queues_across_pause_and_reconfiguration() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        // The kernel-thread fixture configures the device directly; real
        // SET_OWNER and fd lookup are covered by the userspace device tests.
        let (device, call) = create_device(memory.clone());
        let mut kick = device.kick_event(0).unwrap().clone();
        let err = device.queues[0].err.as_ref().unwrap().clone();
        let mut echo = EchoDevice::new(device);
        echo.worker.activate().unwrap();
        echo.worker.activate().unwrap();
        wait_event(&echo.idle);
        for (index, payload) in [*b"echo", *b"next"].into_iter().enumerate() {
            memory
                .write_owner_val(
                    DESC_ADDR,
                    &create_descriptor(GUEST_ADDR, 4, DescFlags::NEXT, 1),
                )
                .unwrap();
            memory
                .write_owner_val(
                    DESC_ADDR + size_of::<Descriptor>(),
                    &create_descriptor(GUEST_ADDR + 4, 4, DescFlags::WRITE, 0),
                )
                .unwrap();
            memory.write_owner_bytes(GUEST_UVA, &payload).unwrap();
            memory
                .write_owner_val(AVAIL_ADDR + AvailRing::entry_offset(index).unwrap(), &0u16)
                .unwrap();
            atomic::fence(Ordering::Release);
            memory
                .write_owner_val(AVAIL_ADDR + AvailRing::IDX_OFFSET, &((index + 1) as u16))
                .unwrap();
            kick.signal();
            if index == 1 {
                assert!(!echo.common.lock().is_running());
                assert_eq!(echo.common.lock().queue_base(0).unwrap(), 1);
                echo.worker.activate().unwrap();
            }
            wait_event(&call);
            let mut response = [0; 4];
            memory
                .read_owner_bytes(GUEST_UVA + 4, &mut response)
                .unwrap();
            assert_eq!(response, payload);
            assert_eq!(
                echo.common.lock().queue_base(0).unwrap(),
                (index + 1) as u32
            );
            assert!(echo.common.lock().is_running());
            wait_event(&echo.idle);
            if index == 0 {
                echo.worker.deactivate();
                echo.worker.deactivate();
                assert!(!echo.completed.load(Ordering::Acquire));
                assert_eq!(echo.common.lock().queue_base(0).unwrap(), 1);
                kick = create_event();
                echo.common.lock().queues[0].kick = Some(kick.clone());
            }
        }
        echo.worker.stop();
        assert!(echo.completed.load(Ordering::Acquire));
        assert_eq!(err.consume(), None);
        let mut common = echo.common.lock();
        common.reset_owner();
        assert!(!common.is_owned());
        assert!(!common.is_running());
    });
}
