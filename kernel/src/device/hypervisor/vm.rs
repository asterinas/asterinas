// SPDX-License-Identifier: MPL-2.0

use core::fmt::Display;

use ostd::task::Task;

use super::{
    ioctl::{IrqFd, IrqFdConfig, KVM_IRQFD_FLAG_DEASSIGN},
    irqfd::IrqFdBinding,
};
use crate::{
    events::IoEvents,
    fs::{
        file::{
            AccessMode, CreationFlags, FileCommon, FileLike, StatusFlags,
            file_table::{FdFlags, FileDesc, get_file_fast},
        },
        pseudofs::AnonInodeFs,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    syscall::eventfd::EventFile,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

pub(super) struct Vm {
    irqfds: Mutex<Vec<Arc<IrqFdBinding>>>,
}

impl Vm {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            irqfds: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn configure_irqfd(
        self: &Arc<Self>,
        config: IrqFdConfig,
        eventfd: Arc<EventFile>,
    ) -> Result<()> {
        if config.flags & KVM_IRQFD_FLAG_DEASSIGN != 0 {
            let binding = {
                let mut bindings = self.irqfds.lock();
                let index = bindings
                    .iter()
                    .position(|binding| binding.matches(&eventfd, config.gsi));
                index.map(|index| bindings.remove(index))
            };
            if let Some(binding) = binding {
                binding.deactivate();
            }
            return Ok(());
        }

        let binding = IrqFdBinding::new(eventfd.clone(), config.gsi, Arc::downgrade(self));
        {
            let mut bindings = self.irqfds.lock();
            if bindings
                .iter()
                .any(|binding| binding.uses_eventfd(&eventfd))
            {
                return_errno_with_message!(Errno::EBUSY, "eventfd already has an irqfd binding");
            }
            bindings.push(binding.clone());
        }
        binding.start();
        Ok(())
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        let bindings = core::mem::take(self.irqfds.get_mut());
        for binding in bindings {
            binding.deactivate();
        }
    }
}

pub(super) struct VmFile {
    vm: Arc<Vm>,
    common: FileCommon,
}

impl VmFile {
    pub(super) fn new(vm: Arc<Vm>) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[kvm-vm]".to_string());
        Self {
            vm,
            common: FileCommon::new(pseudo_path, StatusFlags::empty()),
        }
    }

    fn get_eventfd(raw_fd: i32) -> Result<Arc<EventFile>> {
        let fd = FileDesc::try_from(raw_fd)?;
        let current = Task::current().unwrap();
        let thread_local = current.as_thread_local().unwrap();
        let mut file_table = thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fd).into_owned();
        let file: Arc<dyn Any + Send + Sync> = file;
        file.downcast::<EventFile>()
            .map_err(|_| Error::with_message(Errno::EINVAL, "file descriptor is not an eventfd"))
    }
}

impl Pollable for VmFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for VmFile {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read from VM file");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write to VM file");
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            cmd @ IrqFd => {
                let config: IrqFdConfig = cmd.read()?;
                let eventfd = Self::get_eventfd(i32::try_from(config.fd)?)?;
                self.vm.configure_irqfd(config, eventfd)?;
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "unknown KVM VM ioctl command"),
        })
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<VmFile>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }

        Box::new(FdInfo {
            inner: self,
            fd_flags,
        })
    }
}
