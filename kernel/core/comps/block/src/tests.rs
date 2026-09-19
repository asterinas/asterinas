// SPDX-License-Identifier: MPL-2.0

use ::device_id::MinorId;
use ostd::prelude::ktest;

use super::*;

#[derive(Debug)]
struct TestDevice {
    id: DeviceId,
    name: &'static str,
    is_partition: bool,
    requests: AtomicUsize,
}

impl BlockDevice for TestDevice {
    fn enqueue(&self, _: SubmittedBio) -> Result<(), BioEnqueueError> {
        unreachable!("registry tests do not submit BIOs")
    }

    fn metadata(&self) -> BlockDeviceMeta {
        BlockDeviceMeta::default()
    }

    fn name(&self) -> &str {
        self.name
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn is_partition(&self) -> bool {
        self.is_partition
    }
}

impl BlockRequestHandler for TestDevice {
    fn handle_next_request(&self) {
        // A handler must be callable without holding the registry lock.
        let _ = collect_all();
        self.requests.fetch_add(1, Ordering::Relaxed);
    }
}

#[ktest]
fn request_handlers_follow_device_registration() {
    let major = allocate_major().unwrap();
    let devices: Vec<_> = ["worker-low", "ordinary", "worker-high", "partition"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            Arc::new(TestDevice {
                id: DeviceId::new(major.get(), MinorId::new(index as u32)),
                name,
                is_partition: index == 3,
                requests: AtomicUsize::new(0),
            })
        })
        .collect();

    register_with_request_handler(devices[2].clone()).unwrap();
    register(devices[1].clone()).unwrap();
    register_with_request_handler(devices[0].clone()).unwrap();
    register_with_request_handler(devices[3].clone()).unwrap();

    let duplicates: Vec<_> = devices[..3]
        .iter()
        .map(|device| {
            Arc::new(TestDevice {
                id: device.id,
                name: "duplicate",
                is_partition: false,
                requests: AtomicUsize::new(0),
            })
        })
        .collect();

    // Neither registration entry point may replace an existing entry or add a handler.
    for duplicate in &duplicates {
        assert_eq!(register(duplicate.clone()), Err(Error::Registered));
        assert_eq!(
            register_with_request_handler(duplicate.clone()),
            Err(Error::Registered)
        );
    }

    let handlers: Vec<_> = collect_request_handlers()
        .into_iter()
        .filter(|handler| handler.id().major() == major.get())
        .collect();
    assert_eq!(handlers.len(), 2);
    assert_eq!(handlers[0].id(), devices[0].id);
    assert_eq!(handlers[1].id(), devices[2].id);
    for (handler, device) in handlers.iter().zip([&devices[0], &devices[2]]) {
        let expected: Arc<dyn BlockRequestHandler> = device.clone();
        assert!(Arc::ptr_eq(handler, &expected));
        assert_eq!(device.requests.load(Ordering::Relaxed), 0);
        handler.handle_next_request();
        assert_eq!(device.requests.load(Ordering::Relaxed), 1);
    }
    for duplicate in &duplicates {
        assert_eq!(duplicate.requests.load(Ordering::Relaxed), 0);
    }

    for device in &devices {
        let expected: Arc<dyn BlockDevice> = device.clone();
        assert!(Arc::ptr_eq(&lookup(device.id).unwrap(), &expected));
        assert!(Arc::ptr_eq(
            &lookup_by_name(device.name).unwrap(),
            &expected
        ));
        assert!(
            collect_all()
                .iter()
                .any(|entry| Arc::ptr_eq(entry, &expected))
        );
        assert!(Arc::ptr_eq(&unregister(device.id).unwrap(), &expected));
        assert!(lookup(device.id).is_none());
        assert_eq!(unregister(device.id).unwrap_err(), Error::NotFound);
    }
    assert!(
        collect_request_handlers()
            .iter()
            .all(|handler| handler.id().major() != major.get())
    );

    // Unregistering removes the entry, but does not revoke an existing worker's Arc.
    handlers[0].handle_next_request();
    assert_eq!(devices[0].requests.load(Ordering::Relaxed), 2);
}
