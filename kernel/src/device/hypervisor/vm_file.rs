// SPDX-License-Identifier: MPL-2.0

//! VM file descriptor implementation

use ostd::{
    mm::{CachePolicy, PAGE_SIZE, PageFlags, PageProperty, vm_space::VmQueriedItem},
    task::Task,
};

use super::{ioctl::*, vcpu_file::VcpuFile, vm::Vm};
use crate::{
    fs::{
        file::{AccessMode, FileLike, file_table::FdFlags},
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::posix_thread::AsThreadLocal,
    util::ioctl::{RawIoctl, dispatch_ioctl},
    vm::vmar::{PageFaultInfo, Vmar},
};

/// VM file descriptor
pub struct VmFile {
    /// VmFile owns the Vm instance, but why 'Arc'?
    /// VcpuFiles need to reference the Vm, but can't act like
    /// struct VcpuFile<'a> { vm: &'a Vm, ... } because the
    /// VcpuFile needs to be 'static to be stored in the file table.
    vm: Arc<Vm>,
    pseudo_path: Path,
}

impl VmFile {
    /// Creates a new VM file
    pub fn new(vm: Arc<Vm>) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[rustshyper-vm]".to_string());
        Self { vm, pseudo_path }
    }

    fn set_user_memory_region(&self, region: UserMemoryRegion) -> Result<()> {
        let vmar = current_vmar()?;
        let memory_size = usize::try_from(region.memory_size)?;
        let userspace_start = usize::try_from(region.userspace_addr)?;
        let guest_start = usize::try_from(region.guest_phys_addr)?;
        let userspace_end = userspace_start
            .checked_add(memory_size)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        let guest_end = guest_start
            .checked_add(memory_size)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;

        let guest_space = self.vm.guest_mem();
        guest_space.record_memory_slot(region.slot, userspace_start, guest_start, memory_size)?;

        let is_readonly = region.flags & KVM_MEM_READONLY != 0;
        let guest_page_flags = if is_readonly {
            PageFlags::RX
        } else {
            PageFlags::RWX
        };
        let prop = PageProperty::new_user(guest_page_flags, CachePolicy::Writeback);

        let mut userspace_addr = userspace_start;
        let mut guest_phys_addr = guest_start;
        while userspace_addr < userspace_end && guest_phys_addr < guest_end {
            let frame = query_user_ram_frame(&vmar, userspace_addr)?;
            let preempt_guard = ostd::task::disable_preempt();
            let mut guest_cursor_mut = guest_space.cursor_mut(
                &preempt_guard,
                &(guest_phys_addr..guest_phys_addr + PAGE_SIZE),
            )?;
            guest_cursor_mut.map(frame, prop);
            userspace_addr += PAGE_SIZE;
            guest_phys_addr += PAGE_SIZE;
        }

        Ok(())
    }
}

fn log_vm_error(context: &str, err: &Error) {
    match err.message() {
        Some(msg) => {
            error!(
                "rustshyper: {} failed: errno={:?}, msg={}",
                context,
                err.error(),
                msg
            );
        }
        None => {
            error!("rustshyper: {} failed: errno={:?}", context, err.error());
        }
    }
}

impl FileLike for VmFile {
    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read from VM file");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write to VM file");
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            CreateVcpu => {
                let vcpu_id = u32::try_from(raw_ioctl.arg())?;

                // Create the VCPU
                let vcpu = self.vm.create_vcpu(vcpu_id)?;

                // Create a file descriptor for the VCPU
                let vcpu_file = Arc::new(VcpuFile::new(vcpu));

                // Insert into the current process's file table
                let current = Task::current().unwrap();
                let mut file_table = current.as_thread_local().unwrap().borrow_file_table_mut();
                let mut file_table_locked = file_table.unwrap().write();
                let vcpu_fd = file_table_locked.insert(vcpu_file, FdFlags::empty());

                Ok(vcpu_fd.into())
            }
            cmd @ SetUserMemoryRegion => {
                let region: UserMemoryRegion = cmd.read()?;
                self.set_user_memory_region(region)?;
                Ok(0)
            }
            SetTssAddr => {
                Ok(0)
            }
            _ => {
                let ioctl_nr = raw_ioctl.cmd() & 0xff;
                error!(
                    "rustshyper: unimplemented VM ioctl command: cmd={:#x}, nr={:#x}",
                    raw_ioctl.cmd(),
                    ioctl_nr
                );
                return_errno_with_message!(Errno::ENOTTY, "unknown VM ioctl command");
            }
        })
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn core::fmt::Display> {
        Box::new(alloc::format!("vm_id: {}\n", self.vm.id))
    }
}

fn current_vmar() -> Result<Arc<Vmar>> {
    let current = match Task::current() {
        Some(current) => current,
        None => {
            error!("rustshyper: no current task found for rustshyper ioctl");
            return Err(Error::new(Errno::ESRCH));
        }
    };
    let thread_local = match current.as_thread_local() {
        Some(thread_local) => thread_local,
        None => {
            error!("rustshyper: current task has no ThreadLocal for rustshyper ioctl");
            return Err(Error::new(Errno::EFAULT));
        }
    };
    let vmar = thread_local.vmar().borrow();
    match vmar.as_ref() {
        Some(vmar) => Ok(vmar.clone_arc()),
        None => {
            error!("rustshyper: current thread has no active VMAR for rustshyper ioctl");
            Err(Error::new(Errno::EFAULT))
        }
    }
}

fn query_user_ram_frame(vmar: &Vmar, userspace_addr: Vaddr) -> Result<ostd::mm::UFrame> {
    loop {
        let preempt_guard = ostd::task::disable_preempt();
        let vm_space = vmar.vm_space();
        let mut host_cursor = vm_space.cursor(
            &preempt_guard,
            &(userspace_addr..(userspace_addr + PAGE_SIZE)),
        )?;

        match host_cursor.query()?.1 {
            Some(VmQueriedItem::MappedRam { frame, .. }) => return Ok(frame.clone()),
            Some(VmQueriedItem::MappedIoMem { .. }) => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "guest memory cannot be backed by userspace MMIO"
                );
            }
            None => {}
        }

        drop(host_cursor);
        drop(preempt_guard);

        vmar.handle_page_fault(&PageFaultInfo::new(userspace_addr, PageFlags::R.into()))?;
    }
}

impl crate::process::signal::Pollable for VmFile {
    fn poll(
        &self,
        _mask: crate::events::IoEvents,
        _poller: Option<&mut crate::process::signal::PollHandle>,
    ) -> crate::events::IoEvents {
        // VMs don't support polling
        crate::events::IoEvents::empty()
    }
}
