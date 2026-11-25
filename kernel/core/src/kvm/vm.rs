// SPDX-License-Identifier: MPL-2.0

use super::{vcpu::Vcpu, vm_memory::VmMemory};
use crate::{context::current_userspace, prelude::*, process::VmarSnapshot};

pub(crate) struct Vm {
    _id: u32,
    memory: VmMemory,
    owner_vmar: VmarSnapshot,
    vcpus: Mutex<BTreeMap<u32, Weak<Vcpu>>>,
}

impl Vm {
    pub(crate) fn new(id: u32) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            _id: id,
            memory: VmMemory::new()?,
            owner_vmar: current_userspace!().vmar_snapshot(),
            vcpus: Mutex::new(BTreeMap::new()),
        }))
    }

    pub(crate) fn create_vcpu(self: &Arc<Self>, vcpu_id: u32) -> Result<Arc<Vcpu>> {
        let mut vcpus = self.vcpus.lock();

        if vcpus.get(&vcpu_id).and_then(Weak::upgrade).is_some() {
            return_errno_with_message!(Errno::EEXIST, "vCPU already exists");
        }

        let vcpu = Vcpu::new(vcpu_id, self)?;
        vcpus.insert(vcpu_id, Arc::downgrade(&vcpu));
        Ok(vcpu)
    }

    pub(crate) fn memory(&self) -> &VmMemory {
        &self.memory
    }

    pub(crate) fn check_current_vmar(&self) -> Result<()> {
        let caller_vmar = current_userspace!().vmar_snapshot();
        if !self.owner_vmar.ptr_eq(&caller_vmar) {
            return_errno_with_message!(Errno::EIO, "VM belongs to a different address space");
        }
        Ok(())
    }
}
