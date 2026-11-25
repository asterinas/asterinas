// SPDX-License-Identifier: MPL-2.0

//! Hardware RNG misc-device support.
//!
//! This module registers the `/dev/hwrng` character device and tracks the
//! currently selected [`EntropyDevice`] backend.

use aster_device::{
    class::ClassDevice,
    common::{DevNode, DevNum},
};
use aster_virtio::device::entropy::{self, device::EntropyDevice};
use device_id::{DeviceId, MinorId};

use super::{AnyMiscDevice, MISC_CLASS, MISC_MAJOR};
use crate::{
    device::registry::char,
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

const HWRNG_MINOR: u32 = 183;

/// The currently in-use hardware RNG device.
//
// TODO: Users can select a device by writing its name to `/sys/class/misc/hw_random/rng_current`,
// which is not supported yet.
static RNG_CURRENT: Mutex<Option<Arc<EntropyDevice>>> = Mutex::new(None);

/// The `/dev/hwrng` device.
struct HwRngDevice;

impl AnyMiscDevice for HwRngDevice {
    fn devnode(&self) -> Option<DevNode> {
        Some(DevNode {
            path: Some("hwrng".into()),
            mode: None,
        })
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        // TODO: Reject non-read-only opens with `EINVAL`
        // once device `open` callbacks receive `AccessMode`.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/hw_random/core.c#L169>.
        Ok(Box::new(HwRngFile))
    }
}

/// A file handle opened from `/dev/hwrng`.
struct HwRngFile;

impl Pollable for HwRngFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // Linux's `/dev/hwrng` does not implement `.poll`, so userspace sees the VFS
        // default ("always ready").
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/hw_random/core.c#L287-L292>.
        mask & (IoEvents::IN | IoEvents::OUT)
    }
}

impl FileOps for HwRngFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        // Linux looks up the selected device at `read()`.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/hw_random/core.c#L215>.
        let dev = current_device()?;
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);

        let mut total_copied: usize = 0;
        while writer.avail() > 0 {
            // Clone the writer to keep the cursor at the correct position in case a page fault
            // occurs later.
            let mut new_writer = writer.clone_exclusive();

            let res = if is_nonblocking {
                match dev.try_read(&mut new_writer)? {
                    Some(copied) => Ok(copied),
                    None => return Err(errno!(EAGAIN, "no entropy is available")),
                }
            } else {
                dev.wait_queue()
                    .wait_until(|| match dev.try_read(&mut new_writer) {
                        Ok(Some(copied)) => Some(Ok(copied)),
                        Ok(None) => None,
                        Err(err) => Some(Err(err)),
                    })
            };

            match res {
                Ok(copied) => {
                    writer.skip(copied);
                    total_copied += copied;
                }
                Err(err) => {
                    if total_copied == 0 {
                        return Err(err.into());
                    }
                    break;
                }
            }
        }

        Ok(total_copied)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        // FIXME: Opening this device with `O_WRONLY` or `O_RDWR` fails on Linux. Therefore, the
        // write operation should never be reached. See the TODO in `HwRngDevice::open`.
        Err(errno!(
            EBADF,
            "the hardware RNG device does not support writing"
        ))
    }
}

impl PerOpenFileOps for HwRngFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

fn current_device() -> Result<Arc<EntropyDevice>> {
    let Some(rng) = RNG_CURRENT.lock().clone() else {
        return Err(errno!(ENODEV, "no current hardware RNG device is selected"));
    };
    Ok(rng)
}

pub(super) fn init_in_first_kthread() -> Result<()> {
    if let Some(device) = entropy::first_device() {
        *RNG_CURRENT.lock() = Some(device);
    }

    let class = MISC_CLASS.get().unwrap();
    let id = DeviceId::new(MISC_MAJOR.get().unwrap().get(), MinorId::new(HWRNG_MINOR));
    let device = ClassDevice::builder(class, "hw_random", Box::new(HwRngDevice))
        .devnum(DevNum::char(id))
        .build();

    aster_device::add_device(&device)?;
    if let Err(err) = char::register(device.clone()) {
        let _ = aster_device::remove_device(&device);
        return Err(err);
    }
    Ok(())
}
