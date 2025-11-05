// SPDX-License-Identifier: MPL-2.0

use aster_console::AnyConsoleDevice;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{Infallible, VmReader, VmWriter};
use spin::Once;

use super::{Tty, TtyDriver};
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct ConsoleDriver {
    console: Arc<dyn AnyConsoleDevice>,
}

impl TtyDriver for ConsoleDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L18>.
    const DEVICE_MAJOR_ID: u32 = 4;

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(ConsoleFile(tty)))
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
        self.console.send(chs);
        Ok(chs.len())
    }

    fn drain_output(&self) {}

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        |chs| self.console.send(chs)
    }

    fn can_push(&self) -> bool {
        true
    }

    fn notify_input(&self) {}

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        Some(&*self.console)
    }
}

struct ConsoleFile(Arc<Tty<ConsoleDriver>>);

#[inherit_methods(from = "self.0")]
impl Pollable for ConsoleFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

impl InodeIo for ConsoleFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.read(writer, status_flags)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.write(reader, status_flags)
    }
}

#[inherit_methods(from = "self.0")]
impl FileIo for ConsoleFile {
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a TTY");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

static N_TTY: Once<Box<[Arc<Tty<ConsoleDriver>>]>> = Once::new();

pub(in crate::device) fn init() {
    let devices = {
        let mut devices = aster_console::all_devices();
        // Sort by priorities to ensure that the TTY for the virtio-console device comes first. Is
        // there a better way than hardcoding this?
        devices.sort_by_key(|(name, _)| match name.as_str() {
            aster_virtio::device::console::DEVICE_NAME => 0,
            aster_framebuffer::CONSOLE_NAME => 1,
            _ => 2,
        });
        devices
    };

    let ttys = devices
        .into_iter()
        .enumerate()
        .map(|(index, (_, device))| create_n_tty(index as _, device))
        .collect();
    N_TTY.call_once(|| ttys);
}

fn create_n_tty(index: u32, device: Arc<dyn AnyConsoleDevice>) -> Arc<Tty<ConsoleDriver>> {
    let driver = ConsoleDriver {
        console: device.clone(),
    };

    let tty = Tty::new(index, driver);
    let tty_cloned = tty.clone();

    device.register_callback(Box::leak(Box::new(
        move |mut reader: VmReader<Infallible>| {
            let mut chs = vec![0u8; reader.remain()];
            reader.read(&mut VmWriter::from(chs.as_mut_slice()));
            let _ = tty.push_input(chs.as_slice());
        },
    )));

    tty_cloned
}

/// Returns the system console, i.e., `/dev/console`.
pub fn system_console() -> &'static Arc<Tty<ConsoleDriver>> {
    &N_TTY.get().unwrap()[0]
}

/// Iterates all TTY devices, i.e., `/dev/tty1`, `/dev/tty2`, e.t.c.
pub fn iter_n_tty() -> impl Iterator<Item = &'static Arc<Tty<ConsoleDriver>>> {
    N_TTY.get().unwrap().iter()
}
