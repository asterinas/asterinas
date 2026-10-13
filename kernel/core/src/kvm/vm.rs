use super::{
    ioctl::*,
    vcpu::Vcpu,
    vm_memory::VmMemory,
};
use crate::{prelude::*};
use crate::vm::vmar::{PageFaultInfo, Vmar};
use super::arch::vm::*;

use ostd::{
    mm::{CachePolicy, PageFlags, PageProperty, vm_space::VmQueriedItem},
    task::Task,
    vm::Gpaddr,
};

pub struct Vm {
    pub id: u32,
    memory: VmMemory,
    vcpus: Mutex<BTreeMap<u32, Arc<Vcpu>>>,
    arch: VmArch,
}

impl Vm {
    pub fn new(id: u32) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            id,
            memory: VmMemory::new()?,
            vcpus: Mutex::new(BTreeMap::new()),
            arch: VmArch::new(),
        }))
    }

    pub fn init(&self){
        self.arch.init();
    }

    pub fn memory(&self) -> &VmMemory {
        &self.memory
    }

    pub fn create_vcpu(self: &Arc<Self>, vcpu_id: u32) -> Result<Arc<Vcpu>> {
        let mut vcpus = self.vcpus.lock();
        if vcpus.contains_key(&vcpu_id) {
            return_errno_with_message!(Errno::EEXIST, "vCPU already exists");
        }

        let vcpu = Vcpu::new(vcpu_id, self)?;
        vcpus.insert(vcpu_id, vcpu.clone());
        drop(vcpus);
        Ok(vcpu)
    }

    pub fn set_user_memory_region(&self, region: UserMemoryRegion) -> Result<()> {
        let memory_size = usize::try_from(region.memory_size)?;
        if region.flags & !KVM_MEM_READONLY != 0 {
            return_errno_with_message!(Errno::EINVAL, "unsupported guest memory flags");
        }
        if memory_size == 0 {
            self.memory().set_region(
                region.slot,
                0,
                0,
                Vec::new(),
                default_guest_mem_prop(false),
            )?;
            return Ok(());
        }

        let vmar = current_vmar()?;
        let userspace_start = usize::try_from(region.userspace_addr)?;
        let guest_start = usize::try_from(region.guest_phys_addr)?;
        let userspace_end = userspace_start
            .checked_add(memory_size)
            .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        validate_user_memory_region(userspace_start, guest_start, memory_size)?;

        let mut frames = Vec::new();
        let mut userspace_addr = userspace_start;
        while userspace_addr < userspace_end {
            frames.push(query_user_ram_frame(&vmar, userspace_addr)?);
            userspace_addr += PAGE_SIZE;
        }

        let prop = default_guest_mem_prop(region.flags & KVM_MEM_READONLY != 0);
        self.memory()
            .set_region(region.slot, guest_start, memory_size, frames, prop)?;

        Ok(())
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        debug!("kvm: release VM {}.", self.id);
    }
}

fn default_guest_mem_prop(is_readonly: bool) -> PageProperty {
    let guest_page_flags = if is_readonly {
        PageFlags::RX
    } else {
        PageFlags::RWX
    };
    PageProperty::new_user(guest_page_flags, CachePolicy::Writeback)
}

fn validate_user_memory_region(
    userspace_start: Vaddr,
    guest_start: Gpaddr,
    memory_size: usize,
) -> Result<()> {
    if !userspace_start.is_multiple_of(PAGE_SIZE)
        || !guest_start.is_multiple_of(PAGE_SIZE)
        || !memory_size.is_multiple_of(PAGE_SIZE)
    {
        return_errno_with_message!(Errno::EINVAL, "guest memory region must be page-aligned");
    }
    Ok(())
}

fn current_vmar() -> Result<Arc<Vmar>> {
    let current = match Task::current() {
        Some(current) => current,
        None => {
            error!("kvm: no current task found for hypervisor ioctl");
            return Err(Error::new(Errno::ESRCH));
        }
    };
    let thread_local = match current.as_thread_local() {
        Some(thread_local) => thread_local,
        None => {
            error!("kvm: current task has no ThreadLocal for hypervisor ioctl");
            return Err(Error::new(Errno::EFAULT));
        }
    };
    let vmar = thread_local.vmar().borrow();
    match vmar.as_ref() {
        Some(vmar) => Ok(vmar.clone_arc()),
        None => {
            error!("kvm: current thread has no active VMAR for hypervisor ioctl");
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
