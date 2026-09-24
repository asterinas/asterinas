// SPDX-License-Identifier: MPL-2.0

//! Hardware RNG misc-device support.
//!
//! This module registers the `/dev/hwrng` character device and tracks the
//! currently selected [`EntropyDevice`] backend.

use aster_virtio::device::entropy::{self, device::EntropyDevice};

use super::{MiscDevice, MiscDeviceFile, register_misc_device};
use crate::prelude::*;

const HWRNG_MINOR: u32 = 183;

/// The currently in-use hardware RNG device.
//
// TODO: Users can select a device by writing its name to `/sys/class/misc/hw_random/rng_current`,
// which is not supported yet.
static RNG_CURRENT: Mutex<Option<Arc<EntropyDevice>>> = Mutex::new(None);

/// The `/dev/hwrng` device.
struct HwRngDevice;

impl MiscDevice for HwRngDevice {
    fn open(&self) -> Result<Box<dyn MiscDeviceFile>> {
        // TODO: Reject non-read-only opens with `EINVAL`
        // once device `open` callbacks receive `AccessMode`.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/hw_random/core.c#L169>.
        Ok(Box::new(HwRngFile))
    }
}

/// A file handle opened from `/dev/hwrng`.
struct HwRngFile;

impl MiscDeviceFile for HwRngFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn read(&self, writer: &mut VmWriter, is_nonblocking: bool) -> Result<usize> {
        // Linux looks up the selected device at `read()`.
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/char/hw_random/core.c#L215>.
        let dev = current_device()?;

        let mut total_copied: usize = 0;
        while writer.avail() > 0 {
            // Clone the writer to keep the cursor at the correct position in case a page fault
            // occurs later.
            let mut new_writer = writer.clone_exclusive();

            let res = if is_nonblocking {
                match dev.try_read(&mut new_writer)? {
                    Some(copied) => Ok(copied),
                    None => return_errno_with_message!(Errno::EAGAIN, "no entropy is available"),
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

    fn write(&self, _reader: &mut VmReader, _is_nonblocking: bool) -> Result<usize> {
        // FIXME: Opening this device with `O_WRONLY` or `O_RDWR` fails on Linux. Therefore, the
        // write operation should never be reached. See the TODO in `HwRngDevice::open`.
        return_errno_with_message!(
            Errno::EBADF,
            "the hardware RNG device does not support writing"
        );
    }
}

fn current_device() -> Result<Arc<EntropyDevice>> {
    let Some(rng) = RNG_CURRENT.lock().clone() else {
        return_errno_with_message!(Errno::ENODEV, "no current hardware RNG device is selected");
    };
    Ok(rng)
}

pub(super) fn init_in_first_kthread() {
    if let Some(device) = entropy::first_device() {
        *RNG_CURRENT.lock() = Some(device);
    }

    register_misc_device(HWRNG_MINOR, "hwrng", Arc::new(HwRngDevice)).unwrap();
}
