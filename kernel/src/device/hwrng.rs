// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::String, sync::Arc};

use aster_virtio::device::entropy::{
    all_devices,
    device::{EntropyDevice, RNG_CURRENT},
    get_device, register_recv_callback,
};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::mm::{VmReader, VmWriter, io_util::HasVmReaderWriter};

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
    util::ring_buffer::RingBuffer,
};

static HW_RNG_HANDLE: Mutex<Option<Arc<HwRngHandle>>> = Mutex::new(None);

const HW_RNG_BUFFER_CAPACITY: usize = 4096;

#[derive(Clone)]
struct HwRngHandle {
    rng: Arc<EntropyDevice>,
    pollee: Pollee,
    recv_state: Arc<SpinLock<HwRngRecvState>>,
}

struct HwRngRecvState {
    buffer: RingBuffer<u8>,
    in_flight: bool,
}

impl HwRngHandle {
    pub fn new(rng: Arc<EntropyDevice>) -> Self {
        Self {
            rng,
            pollee: Pollee::new(),
            recv_state: Arc::new(SpinLock::new(HwRngRecvState {
                buffer: RingBuffer::new(HW_RNG_BUFFER_CAPACITY),
                in_flight: false,
            })),
        }
    }

    pub fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();

        if !self.recv_state.lock().buffer.is_empty() {
            events |= IoEvents::IN;
        }

        events
    }

    fn activate_receive_buffer(&self) {
        let should_activate = {
            let mut state = self.recv_state.disable_irq().lock();
            if state.in_flight || state.buffer.is_full() {
                return;
            }
            state.in_flight = true;
            true
        };

        if should_activate {
            let mut request_queue = self.rng.request_queue.disable_irq().lock();
            self.rng
                .activate_receive_buffer(&mut request_queue, PAGE_SIZE);
        }
    }

    fn handle_recv_irq(&self) {
        let mut request_queue = self.rng.request_queue.disable_irq().lock();
        let Ok((_, used_len)) = request_queue.pop_used() else {
            return;
        };
        drop(request_queue);

        let used_len = used_len as usize;
        self.rng
            .receive_buffer
            .sync_from_device(0..used_len)
            .unwrap();

        let (wrote, should_activate) = {
            let mut state = self.recv_state.disable_irq().lock();
            let free_len = state.buffer.free_len();
            let read_len = used_len.min(free_len);

            let mut wrote = 0;
            if read_len > 0 {
                let mut reader = self.rng.receive_buffer.reader().unwrap();
                reader.limit(read_len);

                let mut tmp = vec![0u8; read_len];
                let mut writer = VmWriter::from(tmp.as_mut_slice());
                wrote = reader.read(&mut writer);
                state.buffer.push_slice(&tmp[..wrote]).unwrap();
            }
            state.in_flight = false;

            let should_activate = if state.buffer.is_full() {
                false
            } else {
                state.in_flight = true;
                true
            };

            (wrote, should_activate)
        };

        if wrote > 0 {
            self.pollee.notify(IoEvents::IN);
        }

        if should_activate {
            let mut request_queue = self.rng.request_queue.disable_irq().lock();
            self.rng
                .activate_receive_buffer(&mut request_queue, PAGE_SIZE);
        }
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut state = self.recv_state.disable_irq().lock();
        if state.buffer.is_empty() {
            return_errno_with_message!(Errno::EAGAIN, "entropy buffer is not ready");
        }

        state.buffer.read_fallible(writer)
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
            self.activate_receive_buffer();

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
