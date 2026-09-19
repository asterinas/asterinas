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
    common: Mutex<VhostSession<1>>,
    shared: Arc<VhostSharedData<1>>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl EchoDevice {
    fn new(mut device: VhostRuntimeData<1>) -> Self {
        device.disable_queues();
        let mut session = VhostSession::new(CONFIG);
        let shared = session.shared().clone();
        *shared.lock() = device;
        let idle = create_event();
        let completed = Arc::new(AtomicBool::new(false));
        session
            .start_worker(EchoWork {
                shared: shared.clone(),
                idle: idle.clone(),
                completed: completed.clone(),
            })
            .unwrap();
        Self {
            common: Mutex::new(session),
            shared,
            idle,
            completed,
        }
    }
}

struct EchoWork {
    shared: Arc<VhostSharedData<1>>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl VhostWork for EchoWork {
    fn process(&mut self) -> VhostWorkStatus {
        let mut device = self.shared.lock();
        let status = if device.is_running() {
            match Self::process_queue(&mut device) {
                Ok(status) => status,
                Err(_) => {
                    let (_, queues) = device.memory_and_queues_mut().unwrap();
                    queues[0].signal_error();
                    VhostWorkStatus::Idle
                }
            }
        } else {
            VhostWorkStatus::Idle
        };
        drop(device);
        if status == VhostWorkStatus::Idle {
            self.idle.signal();
        }
        status
    }
}

impl Drop for EchoWork {
    fn drop(&mut self) {
        self.completed.store(true, Ordering::Release);
    }
}

impl EchoWork {
    fn process_queue(device: &mut VhostRuntimeData<1>) -> Result<VhostWorkStatus> {
        Ok({
            let features = Feature::from_bits_truncate(device.negotiated_features());
            let (memory, queues) = device.memory_and_queues_mut()?;
            let queue = &mut queues[0];
            queue.disable_kick_notifications(memory)?;
            if Self::copy_next(queue, memory, features)? {
                queue.notify(memory)?;
                VhostWorkStatus::Pending
            } else if queue.enable_kick_notifications(memory)? {
                VhostWorkStatus::Pending
            } else {
                VhostWorkStatus::Idle
            }
        })
    }

    fn copy_next(
        queue: &mut VhostVirtQueue,
        memory: &VhostMemorySpace,
        features: Feature,
    ) -> Result<bool> {
        let Some(chain) = queue.try_pop(memory, features)? else {
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
                assert!(!echo.shared.lock().is_running());
                assert_eq!(echo.shared.lock().queue_base(0).unwrap(), 1);
                echo.common.lock().enable_queues().unwrap();
            }
            wait_event(&call);
            let mut response = [0; 4];
            memory
                .read_owner_bytes(GUEST_UVA + 4, &mut response)
                .unwrap();
            assert_eq!(response, payload);
            assert_eq!(
                echo.shared.lock().queue_base(0).unwrap(),
                (index + 1) as u32
            );
            assert!(echo.shared.lock().is_running());
            wait_event(&echo.idle);
            if index == 0 {
                echo.common.lock().disable_queues();
                echo.common.lock().disable_queues();
                assert!(!echo.completed.load(Ordering::Acquire));
                assert_eq!(echo.shared.lock().queue_base(0).unwrap(), 1);
                kick = create_event();
                echo.shared.lock().queues[0].set_kick(Some(kick.clone()));
            }
        }
        // A malformed head ends this batch, but the worker stays alive and
        // retries the same available entry after the guest repairs it.
        memory
            .write_owner_val(
                AVAIL_ADDR + AvailRing::entry_offset(2).unwrap(),
                &(QUEUE_SIZE as u16),
            )
            .unwrap();
        memory
            .write_owner_val(AVAIL_ADDR + AvailRing::IDX_OFFSET, &3u16)
            .unwrap();
        kick.signal();
        wait_event(&err);
        wait_event(&echo.idle);
        assert!(!echo.completed.load(Ordering::Acquire));
        assert_eq!(echo.shared.lock().queue_base(0).unwrap(), 2);
        memory
            .write_owner_val(AVAIL_ADDR + AvailRing::entry_offset(2).unwrap(), &0u16)
            .unwrap();
        kick.signal();
        wait_event(&call);
        wait_event(&echo.idle);
        assert_eq!(echo.shared.lock().queue_base(0).unwrap(), 3);
        echo.common.lock().stop_worker();
        assert!(echo.completed.load(Ordering::Acquire));
        echo.common.lock().reset_owner();
        let common = echo.shared.lock();
        assert!(!common.is_owned());
        assert!(!common.is_running());
    });
}

#[ktest]
fn vhost_worker_rechecks_pause_between_kick_registration_and_processing() {
    struct PausedWork {
        echo: EchoWork,
        entered: Arc<KernelEventFile>,
        resume: Arc<KernelEventFile>,
        first: bool,
    }

    impl VhostWork for PausedWork {
        fn process(&mut self) -> VhostWorkStatus {
            if self.first {
                self.first = false;
                self.entered.signal();
                wait_event(&self.resume);
            }
            self.echo.process()
        }
    }

    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let (runtime, call) = create_device(memory.clone());
        let mut session = VhostSession::new(CONFIG);
        let shared = session.shared().clone();
        *shared.lock() = runtime;
        let idle = create_event();
        let completed = Arc::new(AtomicBool::new(false));
        let entered = create_event();
        let resume = create_event();
        session
            .start_worker(PausedWork {
                echo: EchoWork {
                    shared: shared.clone(),
                    idle: idle.clone(),
                    completed: completed.clone(),
                },
                entered: entered.clone(),
                resume: resume.clone(),
                first: true,
            })
            .unwrap();
        wait_event(&entered);

        // The common loop registered the old kick, but process has not locked
        // runtime yet. Pause and rebind before allowing it to inspect the ring.
        session.disable_queues();
        let kick = create_event();
        shared.lock().queues[0].set_kick(Some(kick.clone()));
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
        memory.write_owner_bytes(GUEST_UVA, b"race").unwrap();
        make_available(&memory, 0, AvailFlags::empty());
        resume.signal();
        wait_event(&idle);
        assert_eq!(shared.lock().queue_base(0).unwrap(), 0);
        assert_eq!(call.consume(), None);

        session.enable_queues().unwrap();
        wait_event(&call);
        wait_event(&idle);
        assert_eq!(shared.lock().queue_base(0).unwrap(), 1);
        let mut response = [0; 4];
        memory
            .read_owner_bytes(GUEST_UVA + 4, &mut response)
            .unwrap();
        assert_eq!(&response, b"race");

        // Only the replacement kick signals this second request.
        memory
            .write_owner_val(AVAIL_ADDR + AvailRing::entry_offset(1).unwrap(), &0u16)
            .unwrap();
        atomic::fence(Ordering::Release);
        memory
            .write_owner_val(AVAIL_ADDR + AvailRing::IDX_OFFSET, &2u16)
            .unwrap();
        kick.signal();
        wait_event(&call);
        assert_eq!(shared.lock().queue_base(0).unwrap(), 2);
        let weak_shared = Arc::downgrade(&shared);
        drop(shared);
        drop(session);
        assert!(completed.load(Ordering::Acquire));
        assert!(weak_shared.upgrade().is_none());
    });
}
