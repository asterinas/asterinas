// SPDX-License-Identifier: MPL-2.0

//! A four-byte echo backend with persistent queues and a pausable worker.

use core::{sync::atomic, time::Duration};

use super::*;
use crate::{
    events::IoEvents,
    process::signal::{Pollable, Poller},
    thread::Thread,
};

struct EchoDevice {
    common: Arc<Mutex<VhostDevice<1>>>,
    worker: Option<Arc<Thread>>,
    stop: Arc<KernelEventFile>,
    wake: Arc<KernelEventFile>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicU64>,
}

impl EchoDevice {
    fn new(mut device: VhostDevice<1>) -> Self {
        device.deactivate();
        let vmar = device.owner_vmar().unwrap().clone();
        let common = Arc::new(Mutex::new(device));
        let stop = create_event();
        let wake = create_event();
        let idle = create_event();
        let completed = Arc::new(AtomicU64::new(0));
        let worker = {
            let common = common.clone();
            let stop = stop.clone();
            let wake = wake.clone();
            let idle = idle.clone();
            let completed = completed.clone();
            ThreadOptions::new(move || {
                let result = Self::run(&common, &stop, &wake, &idle);
                if result.is_err() {
                    common.lock().queue_mut(0).unwrap().signal_error();
                }
                completed.store(u64::from(result.is_ok()), Ordering::Release);
            })
            .vmar(vmar)
            .spawn()
        };
        Self {
            common,
            worker: Some(worker),
            stop,
            wake,
            idle,
            completed,
        }
    }

    fn set_running(&self, running: bool) -> Result<()> {
        let result = {
            let mut device = self.common.lock();
            if running {
                device.activate()
            } else {
                device.deactivate();
                Ok(())
            }
        };
        self.wake.signal();
        result
    }

    fn run(
        common: &Mutex<VhostDevice<1>>,
        stop: &KernelEventFile,
        wake: &KernelEventFile,
        idle: &KernelEventFile,
    ) -> Result<()> {
        loop {
            let mut poller = Poller::new(None);
            wake.poll(IoEvents::IN, Some(poller.as_handle_mut()));
            if !stop
                .poll(IoEvents::IN, Some(poller.as_handle_mut()))
                .is_empty()
            {
                return Ok(());
            }
            wake.consume();
            let mut device = common.lock();
            if !device.is_running() {
                drop(device);
                idle.signal();
                poller.wait()?;
                continue;
            }
            if let Some(kick) = device.kick_event(0) {
                kick.poll(IoEvents::IN, Some(poller.as_handle_mut()));
            }
            let mut queue = device.queue_mut(0)?;
            queue.consume_kick();
            queue.disable_kick_notifications()?;
            if Self::copy_next(&mut queue)? {
                queue.notify()?;
                drop(device);
                Thread::yield_now();
                continue;
            }
            let retry = queue.enable_kick_notifications()?;
            drop(device);
            if !retry {
                idle.signal();
                poller.wait()?;
            }
        }
    }

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

    fn stop_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.stop.signal();
            worker.join();
        }
    }
}

impl Drop for EchoDevice {
    fn drop(&mut self) {
        self.stop_worker();
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
        let worker = echo.worker.as_ref().unwrap().clone();
        echo.set_running(true).unwrap();
        echo.set_running(true).unwrap();
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
                echo.set_running(true).unwrap();
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
                echo.set_running(false).unwrap();
                echo.set_running(false).unwrap();
                assert!(Arc::ptr_eq(echo.worker.as_ref().unwrap(), &worker));
                assert_eq!(echo.common.lock().queue_base(0).unwrap(), 1);
                kick = create_event();
                echo.common.lock().queues[0].kick = Some(kick.clone());
            }
        }
        echo.stop_worker();
        assert_eq!(echo.completed.load(Ordering::Acquire), 1);
        assert_eq!(err.consume(), None);
        let mut common = echo.common.lock();
        common.reset_owner();
        assert!(!common.is_owned());
        assert!(!common.is_running());
    });
}
