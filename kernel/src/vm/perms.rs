// SPDX-License-Identifier: MPL-2.0

use aster_rights::Rights;
use bitflags::bitflags;
use ostd::mm::PageFlags;

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

impl From<PageFlags> for VmPerms {
    fn from(flags: PageFlags) -> Self {
        let mut perms = VmPerms::empty();
        if flags.contains(PageFlags::R) {
            perms |= VmPerms::READ;
        }
        if flags.contains(PageFlags::W) {
            perms |= VmPerms::WRITE;
        }
        if flags.contains(PageFlags::X) {
            perms |= VmPerms::EXEC;
        }
        perms
    }
}

impl From<VmPerms> for PageFlags {
    fn from(val: VmPerms) -> Self {
        let mut flags = PageFlags::empty();
        if val.contains(VmPerms::READ) {
            flags |= PageFlags::R;
        }
        if val.contains(VmPerms::WRITE) {
            flags |= PageFlags::W;
        }
        if val.contains(VmPerms::EXEC) {
            flags |= PageFlags::X;
        }
        flags
    }
}
