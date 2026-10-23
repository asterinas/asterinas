// SPDX-License-Identifier: MPL-2.0

//! Tests the request-worker rendezvous in both possible registration orders.
//!
//! The test spawner processes one request synchronously so that the tests can
//! observe worker creation without starting kernel threads.

use ::device_id::MinorId;
use ostd::prelude::ktest;

use super::*;

/// A block device that counts invocations of its request handler.
#[derive(Debug)]
struct TestDevice {
    id: DeviceId,
    requests: AtomicUsize,
}

impl TestDevice {
    fn new(id: DeviceId) -> Arc<Self> {
        Arc::new(Self {
            id,
            requests: AtomicUsize::new(0),
        })
    }

    fn registry_entry(device: &Arc<Self>) -> RegisteredDevice {
        RegisteredDevice {
            device: device.clone(),
            request_handler: Some(device.clone()),
        }
    }
}

impl BlockDevice for TestDevice {
    fn enqueue(&self, _: SubmittedBio) -> Result<(), BioEnqueueError> {
        unreachable!("registry tests do not submit BIOs")
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta::default()
    }

    fn name(&self) -> &str {
        "test"
    }

    fn id(&self) -> DeviceId {
        self.id
    }
}

impl BlockRequestHandler for TestDevice {
    fn handle_next_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }
}

fn spawn_test_worker(handler: Arc<dyn BlockRequestHandler>) {
    handler.handle_next_request();
}

#[ktest]
fn spawner_registration_catches_up_existing_device() {
    let major = allocate_major().unwrap();
    let device = TestDevice::new(DeviceId::new(major.get(), MinorId::new(0)));
    let mut registry = DeviceRegistry::new();

    // Registering the device first leaves its request handler pending.
    let request_worker = registry
        .register(TestDevice::registry_entry(&device))
        .unwrap();
    assert!(request_worker.is_none());

    // Registering the spawner returns the previously pending handler.
    let handlers = registry
        .register_request_worker_spawner(spawn_test_worker)
        .unwrap();
    assert_eq!(handlers.len(), 1);
    spawn_test_worker(handlers.into_iter().next().unwrap());
    assert_eq!(device.requests.load(Ordering::Relaxed), 1);
}

#[ktest]
fn device_registration_uses_existing_spawner() {
    let major = allocate_major().unwrap();
    let device = TestDevice::new(DeviceId::new(major.get(), MinorId::new(0)));
    let mut registry = DeviceRegistry::new();

    // Registering the spawner first has no existing handlers to return.
    let handlers = registry
        .register_request_worker_spawner(spawn_test_worker)
        .unwrap();
    assert!(handlers.is_empty());

    // Registering the device returns the existing spawner and its handler.
    let (spawner, handler) = registry
        .register(TestDevice::registry_entry(&device))
        .unwrap()
        .unwrap();
    spawner(handler);
    assert_eq!(device.requests.load(Ordering::Relaxed), 1);
}
