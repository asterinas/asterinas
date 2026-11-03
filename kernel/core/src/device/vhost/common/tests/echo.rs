// SPDX-License-Identifier: MPL-2.0

//! A four-byte echo backend using the common vhost worker.

use core::{
    mem::offset_of,
    sync::atomic::{self, AtomicBool},
    time::Duration,
};

use super::*;
use crate::{
    device::vhost::common::worker::VhostWorkStatus,
    events::IoEvents,
    process::signal::{Pollable, Poller},
};

struct EchoDevice;

impl EchoDevice {
    fn open(&self) -> EchoFile {
        EchoFile {
            common: Mutex::new(VhostFileCommon::new(CONFIG)),
            shared: Arc::new(VhostSharedState::new()),
            idle: create_kernel_event_file(),
            completed: Arc::new(AtomicBool::new(false)),
        }
    }
}

struct EchoFile {
    common: Mutex<VhostFileCommon>,
    shared: Arc<VhostSharedState<1>>,
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl EchoFile {
    fn set_owner(&self, vmar: Arc<Vmar>) -> Result<()> {
        let work = EchoWork {
            idle: self.idle.clone(),
            completed: self.completed.clone(),
        };
        self.common
            .lock()
            .set_owner(&self.shared, vmar, move |shared| work.process(shared))
    }
}

impl Drop for EchoFile {
    fn drop(&mut self) {
        self.common.get_mut().reset_owner(&self.shared);
    }
}

struct EchoWork {
    idle: Arc<KernelEventFile>,
    completed: Arc<AtomicBool>,
}

impl EchoWork {
    fn process(&self, shared: &VhostSharedState<1>) -> VhostWorkStatus {
        let mut device = shared.runtime().lock();
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
    fn process_queue(device: &mut VhostRuntimeState<1>) -> Result<VhostWorkStatus> {
        let features = Feature::from_bits_truncate(device.negotiated_features());
        let (memory, queues) = device.memory_and_queues_mut()?;
        let queue = &mut queues[0];
        queue.disable_kick_notifications(memory)?;
        if Self::copy_next(queue, memory, features)? {
            queue.notify(memory)?;
            Ok(VhostWorkStatus::Pending)
        } else if queue.enable_kick_notifications(memory)? {
            Ok(VhostWorkStatus::Pending)
        } else {
            Ok(VhostWorkStatus::Idle)
        }
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
        let echo = EchoDevice.open();
        echo.set_owner(memory.vmar().clone()).unwrap();
        wait_event(&echo.idle);
        let shared = echo.shared.clone();
        let call = configure_file(&mut echo.common.lock(), &shared);
        let mut kick = shared.runtime().lock().kick_event(0).unwrap().clone();
        let err = create_kernel_event_file();
        shared.runtime().lock().memory_and_queues_mut().unwrap().1[0].set_err(Some(err.clone()));
        shared.enable_queues().unwrap();
        wait_event(&echo.idle);
        for (index, payload) in [*b"echo", *b"next"].into_iter().enumerate() {
            memory
                .write_owner_val(
                    DESC_ADDR,
                    &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT, 1),
                )
                .unwrap();
            memory
                .write_owner_val(
                    DESC_ADDR + size_of::<Descriptor>(),
                    &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::WRITE, 0),
                )
                .unwrap();
            memory.write_owner_bytes(GUEST_UVA, &payload).unwrap();
            memory
                .write_owner_val(AVAIL_ADDR + AvailRing::entry_offset(index).unwrap(), &0u16)
                .unwrap();
            atomic::fence(Ordering::Release);
            memory
                .write_owner_val(
                    AVAIL_ADDR + offset_of!(AvailRing, idx),
                    &((index + 1) as u16),
                )
                .unwrap();
            kick.signal();
            if index == 1 {
                assert!(!shared.runtime().lock().is_running());
                assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 1);
                shared.enable_queues().unwrap();
            }
            wait_event(&call);
            let mut response = [0; 4];
            memory
                .read_owner_bytes(GUEST_UVA + 4, &mut response)
                .unwrap();
            assert_eq!(response, payload);
            assert_eq!(
                shared.runtime().lock().queue_base(0).unwrap(),
                (index + 1) as u32
            );
            assert!(shared.runtime().lock().is_running());
            wait_event(&echo.idle);
            if index == 0 {
                shared.enable_queues().unwrap();
                assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 1);
                shared.disable_queues();
                shared.disable_queues();
                assert!(!echo.completed.load(Ordering::Acquire));
                assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 1);
                kick = create_kernel_event_file();
                shared.runtime().lock().memory_and_queues_mut().unwrap().1[0]
                    .set_kick(Some(kick.clone()));
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
            .write_owner_val(AVAIL_ADDR + offset_of!(AvailRing, idx), &3u16)
            .unwrap();
        kick.signal();
        wait_event(&err);
        wait_event(&echo.idle);
        assert!(!echo.completed.load(Ordering::Acquire));
        assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 2);
        memory
            .write_owner_val(AVAIL_ADDR + AvailRing::entry_offset(2).unwrap(), &0u16)
            .unwrap();
        kick.signal();
        wait_event(&call);
        wait_event(&echo.idle);
        assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 3);
        echo.common.lock().reset_owner(&shared);
        assert!(echo.completed.load(Ordering::Acquire));
        let runtime = shared.runtime().lock();
        assert!(!runtime.is_owned());
        assert!(!runtime.is_running());
        assert_eq!(runtime.negotiated_features(), 0);
    });
}

#[ktest]
fn vhost_worker_rechecks_pause_between_kick_registration_and_processing() {
    crate::thread::init();
    crate::time::clocks::init_for_ktest();
    crate::util::random::init();

    run_with_owner_memory(|memory| {
        let file = EchoDevice.open();
        let shared = file.shared.clone();
        let idle = file.idle.clone();
        let completed = file.completed.clone();
        let entered = create_kernel_event_file();
        let resume = create_kernel_event_file();
        let echo = EchoWork {
            idle: idle.clone(),
            completed: completed.clone(),
        };
        let worker_entered = entered.clone();
        let worker_resume = resume.clone();
        let mut first = true;
        file.common
            .lock()
            .set_owner(&shared, memory.vmar().clone(), move |shared| {
                if first && shared.runtime().lock().is_running() {
                    first = false;
                    worker_entered.signal();
                    wait_event(&worker_resume);
                }
                echo.process(shared)
            })
            .unwrap();
        wait_event(&idle);
        let call = configure_file(&mut file.common.lock(), &shared);
        shared.enable_queues().unwrap();
        wait_event(&entered);

        // The common loop registered the old kick, but process has not locked
        // runtime yet. Pause and rebind before allowing it to inspect the ring.
        shared.disable_queues();
        let kick = create_kernel_event_file();
        shared.runtime().lock().memory_and_queues_mut().unwrap().1[0].set_kick(Some(kick.clone()));
        memory
            .write_owner_val(
                DESC_ADDR,
                &Descriptor::new(GUEST_ADDR, 4, DescFlags::NEXT, 1),
            )
            .unwrap();
        memory
            .write_owner_val(
                DESC_ADDR + size_of::<Descriptor>(),
                &Descriptor::new(GUEST_ADDR + 4, 4, DescFlags::WRITE, 0),
            )
            .unwrap();
        memory.write_owner_bytes(GUEST_UVA, b"race").unwrap();
        make_available(&memory, 0, AvailFlags::empty());
        resume.signal();
        wait_event(&idle);
        assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 0);
        assert_eq!(call.consume(), None);

        shared.enable_queues().unwrap();
        wait_event(&call);
        wait_event(&idle);
        assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 1);
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
            .write_owner_val(AVAIL_ADDR + offset_of!(AvailRing, idx), &2u16)
            .unwrap();
        kick.signal();
        wait_event(&call);
        assert_eq!(shared.runtime().lock().queue_base(0).unwrap(), 2);
        let weak_shared = Arc::downgrade(&shared);
        drop(file);
        assert!(completed.load(Ordering::Acquire));
        let runtime = shared.runtime().lock();
        assert!(!runtime.is_owned());
        assert!(!runtime.is_running());
        assert_eq!(runtime.negotiated_features(), 0);
        assert_eq!(runtime.queue_base(0).unwrap(), 0);
        drop(runtime);
        drop(shared);
        assert!(weak_shared.upgrade().is_none());
    });
}
