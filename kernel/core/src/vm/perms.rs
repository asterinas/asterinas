// SPDX-License-Identifier: MPL-2.0

use aster_rights::Rights;
use bitflags::bitflags;
use ostd::mm::PageFlags;

use crate::prelude::*;

bitflags! {
    /// The memory access permissions of memory mappings.
    // NOTE: `check` hardcodes `MAY_READ >> 3 == READ`, and so for r/w/x bits.
    pub struct VmPerms: u32 {
        /// Readable.
        const READ    = 1 << 0;
        /// Writable.
        const WRITE   = 1 << 1;
        /// Executable.
        const EXEC   = 1 << 2;
        /// May be protected to readable.
        const MAY_READ = 1 << 3;
        /// May be protected to writable.
        const MAY_WRITE = 1 << 4;
        /// May be protected to executable.
        const MAY_EXEC = 1 << 5;
        /// All permissions (READ | WRITE | EXEC).
        const ALL_PERMS = Self::READ.bits | Self::WRITE.bits | Self::EXEC.bits;
        /// All `MAY_*` permissions (MAY_READ | MAY_WRITE | MAY_EXEC).
        const ALL_MAY_PERMS =  Self::MAY_READ.bits | Self::MAY_WRITE.bits | Self::MAY_EXEC.bits;
    }
}

impl VmPerms {
    /// Checks whether all requested permissions (`READ`, `WRITE`, `EXEC`) are
    /// allowed by their corresponding `MAY_*` capabilities.
    pub fn check(&self) -> Result<()> {
        let requested = *self & Self::ALL_PERMS;
        // NOTE: `MAY_READ >> 3 == READ`, and so for r/w/x bits.
        let allowed = VmPerms::from_bits_truncate((*self & Self::ALL_MAY_PERMS).bits >> 3);
        if !allowed.contains(requested) {
            return_errno_with_message!(Errno::EACCES, "permission denied");
        }

        Ok(())
    }

    /// Parses `bits` as requested permissions from user programs and returns errors
    /// if there are unknown permissions.
    pub fn from_user_bits(bits: u32) -> Result<Self> {
        if let Some(vm_perms) = VmPerms::from_bits(bits)
            && Self::ALL_PERMS.contains(vm_perms)
        {
            Ok(vm_perms)
        } else {
            return_errno_with_message!(Errno::EINVAL, "invalid permissions");
        }
    }

    /// Parses `bits` as requested permissions from user programs and ignores any
    /// unknown permissions.
    pub fn from_user_bits_truncate(bits: u32) -> Self {
        VmPerms::from_bits_truncate(bits) & Self::ALL_PERMS
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
