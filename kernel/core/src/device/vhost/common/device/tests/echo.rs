// SPDX-License-Identifier: MPL-2.0

//! A four-byte echo backend using the common vhost worker.

use core::{
    sync::atomic::{self, AtomicBool},
    time::Duration,
};

use super::*;
use crate::{
    device::vhost::common::worker::{VhostWork, VhostWorkStatus},
    events::IoEvents,
    process::signal::{Pollable, Poller},
};

struct EchoDevice {
    common: Arc<VhostDeviceSession<1>>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl EchoDevice {
    fn new(mut device: VhostDeviceData<1>) -> Self {
        device.disable_queues();
        let common = Arc::new(VhostDeviceSession::from_data(device));
        let idle = create_event();
        let completed = Arc::new(AtomicBool::new(false));
        common
            .lock()
            .start_worker(EchoWork {
                common: common.clone(),
                idle: idle.clone(),
                completed: completed.clone(),
            })
            .unwrap();
        Self {
            common,
            idle,
            completed,
        }
    }
}

impl Drop for EchoDevice {
    fn drop(&mut self) {
        self.common.lock().stop_worker();
    }
}

struct EchoWork {
    common: Arc<VhostDeviceSession<1>>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl VhostWork<1> for EchoWork {
    fn process(&mut self, device: &mut VhostDeviceData<1>) -> Result<VhostWorkStatus> {
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
            self.common.lock_data().queue_mut(0).unwrap().signal_error();
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
        let (mut device, call) = create_device(memory.clone());
        let mut kick = device.kick_event(0).unwrap().clone();
        let err = create_event();
        device.queues[0].set_err(Some(err.clone()));
        let echo = EchoDevice::new(device);
        echo.common.lock().enable_queues().unwrap();
        echo.common.lock().enable_queues().unwrap();
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
                assert!(!echo.common.lock_data().is_running());
                assert_eq!(echo.common.lock_data().queue_base(0).unwrap(), 1);
                echo.common.lock().enable_queues().unwrap();
            }
            wait_event(&call);
            let mut response = [0; 4];
            memory
                .read_owner_bytes(GUEST_UVA + 4, &mut response)
                .unwrap();
            assert_eq!(response, payload);
            assert_eq!(
                echo.common.lock_data().queue_base(0).unwrap(),
                (index + 1) as u32
            );
            assert!(echo.common.lock_data().is_running());
            wait_event(&echo.idle);
            if index == 0 {
                echo.common.lock().disable_queues();
                echo.common.lock().disable_queues();
                assert!(!echo.completed.load(Ordering::Acquire));
                assert_eq!(echo.common.lock_data().queue_base(0).unwrap(), 1);
                kick = create_event();
                echo.common.lock_data().queues[0].set_kick(Some(kick.clone()));
            }
        }
        echo.common.lock().stop_worker();
        assert!(echo.completed.load(Ordering::Acquire));
        assert_eq!(err.consume(), None);
        let mut common = echo.common.lock_data();
        common.reset_owner();
        assert!(!common.is_owned());
        assert!(!common.is_running());
    });
}

#[ktest]
fn vhost_worker_requires_owner_and_restarts_after_stop() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let common = Arc::new(VhostDeviceSession::<1>::new(VhostDeviceConfig {
            device_features: Feature::VERSION_1.bits(),
            backend_features: 0,
            max_queue_size: QUEUE_SIZE as u32,
        }));
        let idle = create_event();
        let completed = Arc::new(AtomicBool::new(false));
        let create_work_fn = || EchoWork {
            common: common.clone(),
            idle: idle.clone(),
            completed: completed.clone(),
        };
        let mut device = common.lock();
        assert_eq!(
            device.start_worker(create_work_fn()).unwrap_err().error(),
            Errno::EPERM
        );
        device
            .set_owner(memory.vmar().clone(), create_work_fn())
            .unwrap();
        wait_event(&idle);

        let other_owner = map_owner();
        assert_eq!(
            device
                .set_owner(other_owner.clone_arc(), create_work_fn())
                .unwrap_err()
                .error(),
            Errno::EBUSY
        );
        assert!(Arc::ptr_eq(
            common.lock_data().owner_vmar().unwrap(),
            memory.vmar()
        ));
        assert!(!completed.load(Ordering::Acquire));
        device.stop_worker();
        assert!(completed.load(Ordering::Acquire));

        completed.store(false, Ordering::Release);
        device.start_worker(create_work_fn()).unwrap();
        wait_event(&idle);
        device.stop_worker();
        assert!(completed.load(Ordering::Acquire));
    });
}

#[ktest]
fn vhost_echo_drop_joins_idle_worker_and_releases_device() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (device, _) = create_device(memory);
        let echo = EchoDevice::new(device);
        let weak_device = Arc::downgrade(&echo.common);
        let completed = echo.completed.clone();
        wait_event(&echo.idle);
        assert!(!completed.load(Ordering::Acquire));
        drop(echo);
        assert!(completed.load(Ordering::Acquire));
        assert!(weak_device.upgrade().is_none());
    });
}

#[ktest]
fn vhost_device_drop_joins_idle_worker() {
    struct IdleWork {
        idle: Arc<KernelEventFile>,
        completed: Arc<AtomicBool>,
    }

    impl VhostWork<1> for IdleWork {
        fn process(&mut self, _device: &mut VhostDeviceData<1>) -> Result<VhostWorkStatus> {
            unreachable!("the queues are paused");
        }

        fn after_process(&mut self, _status: VhostWorkStatus) -> Result<()> {
            self.idle.signal();
            Ok(())
        }

        fn on_exit(self, result: Result<()>) {
            result.unwrap();
            self.completed.store(true, Ordering::Release);
        }
    }

    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (mut data, _) = create_device(memory);
        data.disable_queues();
        let device = VhostDeviceSession::from_data(data);
        let idle = create_event();
        let completed = Arc::new(AtomicBool::new(false));
        device
            .lock()
            .start_worker(IdleWork {
                idle: idle.clone(),
                completed: completed.clone(),
            })
            .unwrap();
        wait_event(&idle);
        assert!(!completed.load(Ordering::Acquire));
        drop(device);
        assert!(completed.load(Ordering::Acquire));
    });
}
