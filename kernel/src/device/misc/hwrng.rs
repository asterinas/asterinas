// SPDX-License-Identifier: MPL-2.0

//! Hardware RNG misc-device support.
//!
//! This module registers the `/dev/hwrng` character device and tracks the
//! currently selected hardware RNG provider.

use aster_virtio::device::entropy::{self, device::EntropyDevice};
use device_id::{DeviceId, MinorId};

use crate::{
    device::{Device, DeviceType, DevtmpfsInodeMeta, registry::char},
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::kernel_thread::ThreadOptions,
    util::random,
};

const HWRNG_MINOR: u32 = 183;
const HWRNG_READ_BUFFER_SIZE: usize = 64;
const HWRNG_FEED_BUFFER_SIZE: usize = 64;

enum ReadMode {
    Blocking,
    Nonblocking,
}

/// A hardware RNG provider registered with the hwrng core.
trait HwrngProvider: Send + Sync {
    /// Reads hardware-generated random bytes.
    fn read_bytes(&self, dst: &mut [u8], mode: ReadMode) -> Result<Option<usize>>;
}

impl HwrngProvider for EntropyDevice {
    fn read_bytes(&self, dst: &mut [u8], mode: ReadMode) -> Result<Option<usize>> {
        match mode {
            ReadMode::Nonblocking => Ok(EntropyDevice::try_read_bytes(self, dst)?),
            ReadMode::Blocking => Ok(Some(EntropyDevice::read_bytes(self, dst)?)),
        }
    }
}

/// The currently in-use hardware RNG provider.
//
// TODO: Users can select a device by writing its name to `/sys/class/misc/hw_random/rng_current`,
// which is not supported yet.
static RNG_CURRENT: Mutex<Option<Arc<dyn HwrngProvider>>> = Mutex::new(None);

/// The `/dev/hwrng` device.
#[derive(Debug)]
struct HwRngDevice {
    id: DeviceId,
}

impl HwRngDevice {
    fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        let minor = MinorId::new(HWRNG_MINOR);

        let id = DeviceId::new(major, minor);
        Arc::new(Self { id })
    }
}

impl Device for HwRngDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new("hwrng"))
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
        let provider = current_provider()?;
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);

        let mut total_copied: usize = 0;
        let mut buffer = [0u8; HWRNG_READ_BUFFER_SIZE];
        while writer.avail() > 0 {
            let read_len = writer.avail().min(buffer.len());
            let read_buffer = &mut buffer[..read_len];

            let mode = if is_nonblocking {
                ReadMode::Nonblocking
            } else {
                ReadMode::Blocking
            };

            let copied = match provider.read_bytes(read_buffer, mode) {
                Ok(Some(copied)) => copied,
                Ok(None) => {
                    debug_assert!(is_nonblocking);
                    if total_copied > 0 {
                        return Ok(total_copied);
                    }
                    return_errno_with_message!(Errno::EAGAIN, "no entropy is available");
                }
                Err(err) => {
                    if total_copied > 0 {
                        return Ok(total_copied);
                    }
                    return Err(err);
                }
            };

            match writer.write_fallible(&mut buffer[..copied].into()) {
                Ok(written) => total_copied = total_copied.saturating_add(written),
                Err((err, written)) => {
                    let total_written = total_copied.saturating_add(written);
                    if total_written == 0 {
                        return Err(err.into());
                    }
                    return Ok(total_written);
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
        return_errno_with_message!(
            Errno::EBADF,
            "the hardware RNG device does not support writing"
        );
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

fn current_provider() -> Result<Arc<dyn HwrngProvider>> {
    let Some(rng) = RNG_CURRENT.lock().clone() else {
        return_errno_with_message!(Errno::ENODEV, "no current hardware RNG device is selected");
    };
    Ok(rng)
}

pub(super) fn init_in_first_kthread() {
    if let Some(device) = entropy::first_device() {
        let provider: Arc<dyn HwrngProvider> = device;
        *RNG_CURRENT.lock() = Some(provider.clone());
        start_random_feeder(provider);
    }

    char::register(HwRngDevice::new()).unwrap();
}

fn start_random_feeder(provider: Arc<dyn HwrngProvider>) {
    if random::is_ready() {
        return;
    }

    ThreadOptions::new(move || feed_random_until_ready(provider)).spawn();
}

fn feed_random_until_ready(provider: Arc<dyn HwrngProvider>) {
    let mut buffer = [0u8; HWRNG_FEED_BUFFER_SIZE];

    while !random::is_ready() {
        let res = provider.read_bytes(&mut buffer, ReadMode::Blocking);

        match res {
            Ok(Some(0)) => {}
            Ok(Some(read_len)) => {
                random::add_entropy(&buffer[..read_len], read_len.saturating_mul(8))
            }
            Ok(None) => {}
            Err(err) => {
                warn!("failed to feed hardware RNG entropy: {err:?}");
                break;
            }
        }
    }
}
