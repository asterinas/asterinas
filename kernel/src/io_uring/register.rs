// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, util::IoVec};

// Matches Linux `IORING_MAX_REG_BUFFERS`.
//
// Reference: <https://elixir.bootlin.com/linux/v7.1.2/source/io_uring/rsrc.c#L34-L35>.
pub(super) const MAX_REGISTERED_BUFFERS: usize = 1 << 14;

pub(super) struct RegisteredResource {
    buffers: Mutex<Option<Box<[IoVec]>>>,
}

impl RegisteredResource {
    pub(super) fn new() -> Self {
        Self {
            buffers: Mutex::new(None),
        }
    }

    pub(super) fn check_buffers_available(&self) -> Result<()> {
        if self.buffers.lock().is_some() {
            return_errno_with_message!(Errno::EBUSY, "io_uring buffers are already registered");
        }

        Ok(())
    }

    pub(super) fn buffer_count(&self) -> usize {
        self.buffers
            .lock()
            .as_ref()
            .map_or(0, |buffers| buffers.len())
    }

    pub(super) fn register_buffers(&self, buffers: Box<[IoVec]>) -> Result<()> {
        let mut registered_buffers = self.buffers.lock();
        if registered_buffers.is_some() {
            return_errno_with_message!(Errno::EBUSY, "io_uring buffers are already registered");
        }

        *registered_buffers = Some(buffers);
        Ok(())
    }

    pub(super) fn unregister_buffers(&self) {
        *self.buffers.lock() = None;
    }

    pub(super) fn get_fixed_buffer(
        &self,
        index: u16,
        addr: Vaddr,
        len: usize,
    ) -> Result<IoVec> {
        let registered_buffers = self.buffers.lock();
        let Some(buffer) = registered_buffers
            .as_ref()
            .and_then(|buffers| buffers.get(index as usize))
        else {
            return_errno_with_message!(Errno::EFAULT, "the fixed buffer is not registered");
        };

        let end = addr
            .checked_add(len)
            .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "the fixed buffer overflows"))?;
        let buffer_end = buffer.base.checked_add(buffer.len).ok_or_else(|| {
            Error::with_message(Errno::EOVERFLOW, "the registered buffer overflows")
        })?;

        if addr < buffer.base || end > buffer_end {
            return_errno_with_message!(Errno::EFAULT, "the fixed buffer range is not registered");
        }

        Ok(IoVec { base: addr, len })
    }
}
