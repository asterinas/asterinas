// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::String, sync::Arc};

use aster_virtio::device::entropy::{
    all_devices,
    device::{EntropyDevice, RNG_CURRENT},
    get_device, register_recv_callback,
};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::mm::{VmReader, VmWriter};

use crate::{
    device::registry::char,
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
};

static HW_RNG_HANDLE: Mutex<Option<Arc<HwRngHandle>>> = Mutex::new(None);

#[derive(Clone)]
struct HwRngHandle {
    rng: Arc<EntropyDevice>,
    pollee: Pollee,
}

impl HwRngHandle {
    pub fn new(rng: Arc<EntropyDevice>) -> Self {
        Self {
            rng,
            pollee: Pollee::new(),
        }
    }

    pub fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.rng.can_pop() {
            events |= IoEvents::IN;
        }

        events
    }

    fn handle_recv_irq(&self) {
        self.pollee.notify(IoEvents::IN);
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.rng
            .try_read(writer)
            .map_err(|(err, _bytes)| Error::from(err))
    }
}

struct HwRngDevice;

impl Device for HwRngDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // Same Value with Linux: major 10, minor 183
        device_id::DeviceId::new(MajorId::new(10), MinorId::new(183))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        Some("hwrng".into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        if RNG_CURRENT.get().is_none() {
            return_errno_with_message!(Errno::ENODEV, "No hardware RNG device found");
        }

        let mut handle_lock = HW_RNG_HANDLE.lock();

        if handle_lock.is_none() {
            let name_lock = RNG_CURRENT.get().unwrap();

            let mut device = get_device(&name_lock.lock());

            if device.is_none() {
                let all = all_devices();
                if let Some((fallback_name, fallback_device)) = all.first() {
                    *name_lock.lock() = fallback_name.clone();
                    device = Some(fallback_device.clone());
                }
            }

            let device = device.ok_or_else(|| {
                Error::with_message(Errno::ENODEV, "No hardware RNG device found")
            })?;

            *handle_lock = Some(Arc::new(HwRngHandle::new(device)));
        }

        let hwrng_handle = handle_lock.as_ref().unwrap();
        Ok(Box::new((**hwrng_handle).clone()))
    }
}

impl Pollable for HwRngHandle {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl InodeIo for HwRngHandle {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let len = writer.avail();
        let mut written_bytes = 0;
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);

        while written_bytes < len {
            let read_once = if is_nonblocking {
                self.try_read(writer)
            } else {
                self.wait_events(IoEvents::IN, None, || self.try_read(writer))
            };

            match read_once {
                Ok(0) => break,
                Ok(copied) => {
                    written_bytes += copied;
                    self.pollee.invalidate();
                }
                Err(err) if is_nonblocking && err.error() == Errno::EAGAIN => {
                    if written_bytes == 0 {
                        return Err(err);
                    }
                    break;
                }
                Err(err) => return Err(err),
            }
        }

        Ok(written_bytes)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let len = reader.remain();
        reader.skip(len);
        Ok(len)
    }
}

impl FileIo for HwRngHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    register_recv_callback(|| {
        let device_lock = HW_RNG_HANDLE.lock();
        if let Some(hwrng_handle) = &*device_lock {
            hwrng_handle.handle_recv_irq();
        }
    });

    char::register(Arc::new(HwRngDevice))?;

    Ok(())
}
