// SPDX-License-Identifier: MPL-2.0

use alloc::{
    format,
    sync::{Arc, Weak},
};

use aster_console::AnyConsoleDevice;
use aster_device::{Device, DeviceId, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, SysAttrSetBuilder, SysBranchNode, SysPerms, SysStr,
};
use inherit_methods_macro::inherit_methods;
use ostd::sync::SpinLock;

use crate::{
    device::{
        pty::UNIX98_PTY_SLAVE_ID_ALLOCATOR,
        tty::{PushCharError, Tty, TtyDriver},
    },
    events::IoEvents,
    prelude::{return_errno_with_message, Errno, Result},
    process::signal::Pollee,
    util::ring_buffer::RingBuffer,
};

const BUFFER_CAPACITY: usize = 8192;

/// A pseudoterminal driver.
///
/// This is contained in the PTY slave, but it maintains the output buffer and the pollee of the
/// master. The pollee of the slave is part of the [`Tty`] structure (see the definition of
/// [`PtySlave`]).
pub struct PtyDriver {
    output: SpinLock<RingBuffer<u8>>,
    pollee: Pollee,
    device: Arc<PtyDevice>,
}

/// A pseudoterminal slave.
pub type PtySlave = Tty<PtyDriver>;

impl PtyDriver {
    pub(super) fn new(index: u32) -> Self {
        Self {
            output: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
            pollee: Pollee::new(),
            device: PtyDevice::new(index),
        }
    }

    pub(super) fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut output = self.output.lock();
        if output.is_empty() {
            return_errno_with_message!(Errno::EAGAIN, "the buffer is empty");
        }

        let read_len = output.len().min(buf.len());
        output.pop_slice(&mut buf[..read_len]).unwrap();

        Ok(read_len)
    }

    pub(super) fn pollee(&self) -> &Pollee {
        &self.pollee
    }

    pub(super) fn buffer_len(&self) -> usize {
        self.output.lock().len()
    }
}

impl TtyDriver for PtyDriver {
    fn push_output(&self, chs: &[u8]) -> core::result::Result<usize, PushCharError> {
        let mut output = self.output.lock();

        let mut len = 0;
        for ch in chs {
            // TODO: This is termios-specific behavior and should be part of the TTY implementation
            // instead of the TTY driver implementation. See the ONLCR flag for more details.
            if *ch == b'\n' && output.capacity() - output.len() >= 2 {
                output.push(b'\r').unwrap();
                output.push(b'\n').unwrap();
            } else if *ch != b'\n' && !output.is_full() {
                output.push(*ch).unwrap();
            } else if len == 0 {
                return Err(PushCharError);
            } else {
                break;
            }
            len += 1;
        }

        self.pollee.notify(IoEvents::IN);
        Ok(len)
    }

    fn drain_output(&self) {
        self.output.lock().clear();
        self.pollee.invalidate();
    }

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        let mut output = self.output.lock();
        let mut has_notified = false;

        move |chs| {
            for ch in chs {
                let _ = output.push(*ch);
            }

            if !has_notified {
                self.pollee.notify(IoEvents::IN);
                has_notified = true;
            }
        }
    }

    fn can_push(&self) -> bool {
        let output = self.output.lock();
        output.capacity() - output.len() >= 2
    }

    fn notify_input(&self) {
        self.pollee.notify(IoEvents::OUT);
    }

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        None
    }

    fn device(&self) -> Arc<dyn Device> {
        self.device.clone()
    }
}

/// The PTY slave device.
#[derive(Debug)]
pub struct PtyDevice {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
}

impl Device for PtyDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> Option<DeviceId> {
        Some(self.id)
    }

    fn sysnode(&self) -> Arc<dyn SysBranchNode> {
        self.weak_self().upgrade().unwrap()
    }
}

inherit_sys_branch_node!(PtyDevice, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[inherit_methods(from = "self.fields")]
impl PtyDevice {
    pub fn weak_self(&self) -> &Weak<Self>;
}

impl PtyDevice {
    pub(super) fn new(index: u32) -> Arc<Self> {
        let id = UNIX98_PTY_SLAVE_ID_ALLOCATOR
            .get()
            .unwrap()
            .allocate(index)
            .unwrap();
        let name = SysStr::from(format!("{}", index));

        let builder = SysAttrSetBuilder::new();
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| PtyDevice {
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }
}

impl Drop for PtyDevice {
    fn drop(&mut self) {
        UNIX98_PTY_SLAVE_ID_ALLOCATOR
            .get()
            .unwrap()
            .release(self.id.minor());
    }
}
