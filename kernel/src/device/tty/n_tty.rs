// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    format,
    sync::{Arc, Weak},
    vec,
};

use aster_console::AnyConsoleDevice;
use aster_device::{Device, DeviceId, DeviceType};
use aster_systree::{
    inherit_sys_branch_node, BranchNodeFields, Error, SysAttrSetBuilder, SysBranchNode, SysObj,
    SysPerms, SysStr,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{Infallible, VmReader, VmWriter};
use spin::Once;

use super::{PushCharError, Tty, TtyDriver};
use crate::{
    device::{tty::TTY_ID_ALLOCATOR, TTYAUX_ID_ALLOCATOR},
    error::Errno,
    events::IoEvents,
    fs::{
        device::{add_device, DeviceFile},
        inode_handle::FileIo,
    },
    prelude::{return_errno_with_message, Result},
    process::signal::{PollHandle, Pollable},
};

/// The `/dev/tty<N>` device.
#[derive(Debug)]
pub struct NttyDevice {
    id: DeviceId,
    fields: BranchNodeFields<dyn SysBranchNode, Self>,
}

impl Device for NttyDevice {
    fn device_type(&self) -> DeviceType {
        DeviceType::Char
    }

    fn device_id(&self) -> Option<DeviceId> {
        Some(self.id)
    }

    fn sysnode(&self) -> Arc<dyn SysBranchNode> {
        self.weak_self().upgrade().unwrap()
    }
}

inherit_sys_branch_node!(NttyDevice, fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        // Check if attribute exists
        if !self.fields.attr_set().contains(name) {
            return Err(Error::NotFound);
        }

        let attr = self.fields.attr_set().get(name).unwrap();
        // Check if attribute is readable
        if !attr.perms().can_read() {
            return Err(Error::PermissionDenied);
        }

        let mut printer = VmPrinter::new_skip(writer, offset);
        if name == "dev" {
            writeln!(printer, "{}:{}", self.id.major(), self.id.minor())
                .map_err(|_| Error::AttributeError)?
        };

        Ok(printer.bytes_written())
    }
});

#[inherit_methods(from = "self.fields")]
impl NttyDevice {
    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    pub fn weak_self(&self) -> &Weak<Self>;
    pub fn child(&self, name: &str) -> Option<Arc<dyn SysBranchNode>>;
    pub fn add_child(&self, new_child: Arc<dyn SysBranchNode>) -> aster_systree::Result<()>;
    pub fn remove_child(&self, child_name: &str) -> aster_systree::Result<Arc<dyn SysBranchNode>>;
}

impl NttyDevice {
    fn new(id: DeviceId, name: SysStr) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        // Add common attributes.
        builder.add(SysStr::from("dev"), SysPerms::DEFAULT_RO_ATTR_PERMS);
        builder.add(SysStr::from("uevent"), SysPerms::DEFAULT_RW_ATTR_PERMS);
        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| NttyDevice {
            id,
            fields: BranchNodeFields::new(name, attrs, weak_self.clone()),
        })
    }
}

impl Drop for NttyDevice {
    fn drop(&mut self) {
        let ida = TTY_ID_ALLOCATOR.get().unwrap();
        if ida.major == self.id.major() {
            ida.release(self.id.minor());
        } else {
            TTYAUX_ID_ALLOCATOR.get().unwrap().release(self.id.minor());
        }
    }
}

pub struct ConsoleDriver {
    console: Arc<dyn AnyConsoleDevice>,
    device: Arc<NttyDevice>,
}

impl ConsoleDriver {
    fn new(index: u32, console: Arc<dyn AnyConsoleDevice>) -> Self {
        let id = TTY_ID_ALLOCATOR.get().unwrap().allocate(index).unwrap();
        let name = SysStr::from(format!("tty{}", index));

        Self {
            console,
            device: NttyDevice::new(id, name),
        }
    }
}

impl TtyDriver for ConsoleDriver {
    fn push_output(&self, chs: &[u8]) -> core::result::Result<usize, PushCharError> {
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

    fn set_font(&self, font: aster_console::BitmapFont) -> Result<()> {
        use aster_console::ConsoleSetFontError;

        match self.console.set_font(font) {
            Ok(()) => Ok(()),
            Err(ConsoleSetFontError::InappropriateDevice) => {
                return_errno_with_message!(
                    Errno::ENOTTY,
                    "the console has no support for font setting"
                )
            }
            Err(ConsoleSetFontError::InvalidFont) => {
                return_errno_with_message!(Errno::EINVAL, "the font is invalid for the console")
            }
        }
    }

    fn as_device(&self) -> Arc<dyn Device> {
        self.device.clone()
    }
}

static DEV_CONSOLE: Once<Arc<DevConsole>> = Once::new();

/// The `/dev/console` device.
#[derive(Debug)]
pub struct DevConsole {
    inner: Arc<NttyDevice>,
    tty: Arc<Tty<ConsoleDriver>>,
}

impl Device for DevConsole {
    fn device_type(&self) -> DeviceType {
        self.inner.device_type()
    }

    fn device_id(&self) -> Option<DeviceId> {
        self.inner.device_id()
    }

    fn sysnode(&self) -> Arc<dyn SysBranchNode> {
        self.inner.sysnode()
    }
}

impl Pollable for DevConsole {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.tty.poll(mask, poller)
    }
}

impl FileIo for DevConsole {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.tty.read(writer)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.tty.write(reader)
    }

    fn ioctl(&self, cmd: crate::fs::utils::IoctlCmd, arg: usize) -> Result<i32> {
        self.tty.ioctl(cmd, arg)
    }

    fn mappable(&self) -> Result<crate::fs::file_handle::Mappable> {
        self.tty.mappable()
    }
}

impl DeviceFile for DevConsole {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        self.tty.open()
    }
}

static N_TTY: Once<Box<[Arc<Tty<ConsoleDriver>>]>> = Once::new();

pub(in crate::device) fn init_in_first_process() {
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

    let id = TTYAUX_ID_ALLOCATOR.get().unwrap().allocate(1).unwrap();
    let device = NttyDevice::new(id, SysStr::from("console"));
    let dev_console = Arc::new(DevConsole {
        inner: device,
        tty: system_console().clone(),
    });
    add_device(dev_console.clone());
    DEV_CONSOLE.call_once(|| dev_console);
}

fn create_n_tty(index: u32, device: Arc<dyn AnyConsoleDevice>) -> Arc<Tty<ConsoleDriver>> {
    let driver = ConsoleDriver::new(index, device.clone());

    let tty = Tty::new(index, driver);
    let tty_cloned = tty.clone();

    add_device(tty.clone());

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
