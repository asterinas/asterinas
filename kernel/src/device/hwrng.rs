// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::String, sync::Arc};

use aster_virtio::device::entropy::{
    all_devices, device::EntropyDevice, get_device, register_recv_callback,
};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::{
    mm::{FallibleVmRead, VmReader, VmWriter},
    sync::WaitQueue,
};

use crate::{
    device::{Device, DeviceType, registry::char},
    events::IoEvents,
    fs::{
        file::{FileIo, StatusFlags},
        vfs::inode::InodeIo,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// A `WaitQueue` for data notification from hardware RNG devices.
//
// TODO: Ideally, each device should have its own `WaitQueue`. However, this queue is shared by all
// hardware RNG devices. This applies even if the device is not currently in use.
static RNG_WAIT_QUEUE: WaitQueue = WaitQueue::new();
/// The name of the currently in-use hardware RNG device.
//
// TODO: Users can select a device by writing the name to `/sys/class/misc/hw_random/rng_current`,
// which is not supported yet.
static RNG_CURRENT_NAME: Mutex<Option<String>> = Mutex::new(None);

struct RngCurrent;

impl RngCurrent {
    fn current_device() -> Result<Arc<EntropyDevice>> {
        let Some(name) = RNG_CURRENT_NAME.lock().clone() else {
            return_errno_with_message!(Errno::ENODEV, "no current hardware RNG device is selected");
        };

        let Some(rng) = get_device(&name) else {
            return_errno_with_message!(
                Errno::ENODEV,
                "the current hardware RNG device is unavailable"
            );
        };

        Ok(rng)
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let rng = Self::current_device()?;
        let Some((read_buffer, read_buffer_len)) = rng.try_read() else {
            return_errno_with_message!(Errno::EAGAIN, "no random data is available");
        };

        // If `read_buffer` has more bytes than the writer, we'll drop the trailing bytes. This
        // should be fine, since we're just dropping random data.
        Ok(read_buffer
            .reader()
            .unwrap()
            .limit(read_buffer_len)
            .read_fallible(writer)?)
    }
}

impl Pollable for RngCurrent {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        mask & (IoEvents::IN | IoEvents::OUT)
    }
}

impl InodeIo for RngCurrent {
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
            let read_res = if is_nonblocking {
                self.try_read(writer)
            } else {
                RNG_WAIT_QUEUE
                    .pause_until(|| match self.try_read(writer) {
                        Ok(copied) => Some(Ok(copied)),
                        Err(err) if err.error() == Errno::EAGAIN => None,
                        Err(err) => Some(Err(err)),
                    })
                    .flatten()
            };

            match read_res {
                Ok(copied) => {
                    written_bytes += copied;
                }
                Err(err) if written_bytes == 0 => return Err(err),
                Err(_) => break,
            }
        }

        Ok(written_bytes)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(
            Errno::EBADF,
            "the hardware RNG device does not support writing"
        );
    }
}

impl FileIo for RngCurrent {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

struct HwRngDevice;

impl Device for HwRngDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux: major 10, minor 183.
        device_id::DeviceId::new(MajorId::new(10), MinorId::new(183))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        Some("hwrng".into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(RngCurrent))
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    if let Some((name, _)) = all_devices().into_iter().next() {
        *RNG_CURRENT_NAME.lock() = Some(name);
    }

    register_recv_callback(|| {
        RNG_WAIT_QUEUE.wake_all();
    });

    char::register(Arc::new(HwRngDevice))?;

    Ok(())
}
