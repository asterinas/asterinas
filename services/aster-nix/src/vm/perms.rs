// SPDX-License-Identifier: MPL-2.0

use aster_frame::vm::VmPerm;
use aster_rights::Rights;
use bitflags::bitflags;

bitflags! {
    /// The memory access permissions of memory mappings.
    pub struct VmPerms: u32 {
        /// Readable.
        const READ    = 1 << 0;
        /// Writable.
        const WRITE   = 1 << 1;
        /// Executable.
        const EXEC   = 1 << 2;
    }
}

impl From<Rights> for VmPerms {
    fn from(rights: Rights) -> VmPerms {
        let mut vm_perm = VmPerms::empty();
        if rights.contains(Rights::READ) {
            vm_perm |= VmPerms::READ;
        }
        if rights.contains(Rights::WRITE) {
            vm_perm |= VmPerms::WRITE;
        }
        if rights.contains(Rights::EXEC) {
            vm_perm |= VmPerms::EXEC;
        }
        vm_perm
    }
}

impl From<VmPerms> for Rights {
    fn from(vm_perms: VmPerms) -> Rights {
        let mut rights = Rights::empty();
        if vm_perms.contains(VmPerms::READ) {
            rights |= Rights::READ;
        }
        if vm_perms.contains(VmPerms::WRITE) {
            rights |= Rights::WRITE;
        }
        if vm_perms.contains(VmPerms::EXEC) {
            rights |= Rights::EXEC;
        }
        rights
    }
}

impl From<VmPerm> for VmPerms {
    fn from(perm: VmPerm) -> Self {
        let mut perms = VmPerms::empty();
        if perm.contains(VmPerm::R) {
            perms |= VmPerms::READ;
        }
        if perm.contains(VmPerm::W) {
            perms |= VmPerms::WRITE;
        }
        if perm.contains(VmPerm::X) {
            perms |= VmPerms::EXEC;
        }
        perms
    }
}

impl From<VmPerms> for VmPerm {
    fn from(perms: VmPerms) -> Self {
        let mut perm = VmPerm::empty();
        if perms.contains(VmPerms::READ) {
            perm |= VmPerm::R;
        }
        if perms.contains(VmPerms::WRITE) {
            perm |= VmPerm::W;
        }
        if perms.contains(VmPerms::EXEC) {
            perm |= VmPerm::X;
        }
        perm
    }
}
