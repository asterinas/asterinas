// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmat

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    fs::utils::InodeMode,
    prelude::*,
    vm::{
        perms::VmPerms,
        shared_mem::{SHM_OBJ_MANAGER, SHMLBA},
        vmar::is_userspace_vaddr,
    },
};

bitflags! {
    /// Flags for `shmat()` (shared memory attach) operations.
    pub struct ShmFlags: u32 {
        /// Read-only access (equivalent to `SHM_RDONLY`).
        const RDONLY = 0o10000;
        /// Round attach address to SHMLBA boundary (equivalent to `SHM_RND`).
        const RND    = 0o20000;
        /// Remap existing mapping (equivalent to `SHM_REMAP`).
        const REMAP  = 0o40000;
        /// Execution access (equivalent to `SHM_EXEC`).
        const EXEC   = 0o100000;
    }
}

pub fn sys_shmat(shmid: i32, addr: u64, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
    debug!(
        "[sys_shmat] shmid = {}, addr = {:#x}, flags = {}",
        shmid, addr, flags
    );
    if shmid < 0 {
        return_errno!(Errno::EINVAL);
    }

    let manager = SHM_OBJ_MANAGER.get().ok_or(Errno::EINVAL)?;
    let manager_guard = manager.read();
    let shm_obj = manager_guard
        .get_shm_obj(shmid as u64)
        .ok_or(Errno::EINVAL)?;

    let shm_flags = ShmFlags::from_bits(flags as u32).ok_or(Errno::EINVAL)?;
    let mut vm_perms = VmPerms::empty();
    if shm_flags.contains(ShmFlags::RDONLY) {
        if !shm_obj.mode()?.contains(InodeMode::S_IRUSR) {
            return_errno!(Errno::EACCES);
        }
        vm_perms |= VmPerms::READ;
    } else {
        vm_perms |= VmPerms::READ | VmPerms::WRITE;
    }
    if shm_flags.contains(ShmFlags::EXEC) {
        if !shm_obj.mode()?.contains(InodeMode::S_IXUSR) {
            return_errno!(Errno::EACCES);
        }
        vm_perms |= VmPerms::EXEC;
    }

    let len = shm_obj.size().align_up(PAGE_SIZE);
    let addr = if addr == 0 {
        // If addr is 0, the system chooses the address
        0_usize
    } else if shm_flags.contains(ShmFlags::RND) {
        // If RND is set, align down the address to SHMLBA
        addr.align_down(SHMLBA as u64) as usize
    } else if !addr.is_multiple_of(SHMLBA as u64) {
        // If the address is not aligned with SHMLBA, return error
        return_errno!(Errno::EINVAL);
    } else {
        // Otherwise, use the provided address
        addr as usize
    };

    // Check bounds
    if len == 0 {
        return_errno_with_message!(Errno::EINVAL, "shmat len cannot be zero");
    }
    if len > isize::MAX as usize {
        return_errno_with_message!(Errno::ENOMEM, "shmat len too large");
    }
    if addr > isize::MAX as usize - len {
        return_errno_with_message!(Errno::ENOMEM, "shmat (addr + len) too large");
    }

    // Check fixed address bounds if specified
    if addr != 0 {
        let map_end = addr.checked_add(len).ok_or(Errno::EINVAL)?;
        if !(is_userspace_vaddr(addr) && is_userspace_vaddr(map_end - 1)) {
            return_errno_with_message!(Errno::EINVAL, "Invalid shmat fixed addr");
        }
    }

    // On x86, `PROT_WRITE` implies `PROT_READ`.
    #[cfg(target_arch = "x86_64")]
    let vm_perms = if !vm_perms.contains(VmPerms::READ) && vm_perms.contains(VmPerms::WRITE) {
        vm_perms | VmPerms::READ
    } else {
        vm_perms
    };

    let mut may_perms = VmPerms::empty();
    if vm_perms.contains(VmPerms::READ) {
        may_perms |= VmPerms::MAY_READ;
    }
    if vm_perms.contains(VmPerms::WRITE) {
        may_perms |= VmPerms::MAY_WRITE;
    }
    if vm_perms.contains(VmPerms::EXEC) {
        may_perms |= VmPerms::MAY_EXEC;
    }

    let user_space = ctx.user_space();
    let root_vmar = user_space.vmar();
    if addr != 0 && !shm_flags.contains(ShmFlags::REMAP) {
        let map_end = addr.checked_add(len).ok_or(Errno::EINVAL)?;
        if root_vmar.query(addr..map_end).iter().next().is_some() {
            return_errno!(Errno::EINVAL);
        }
    }

    // Mark shared memory as attached and map it
    let attached_shm = shm_obj.set_attached(ctx.process.pid());

    // Use the manager guard as a global coordinator to avoid deleting the
    // shared memory object during mapping
    drop(manager_guard);

    let vm_map_options = {
        let mut options = root_vmar.new_map(len, vm_perms)?.may_perms(may_perms);
        if addr != 0 {
            options = options
                .offset(addr)
                .can_overwrite(shm_flags.contains(ShmFlags::REMAP));
        }
        options = options.is_shared(true);
        let vmo = shm_obj.vmo();
        options = options
            .vmo(vmo)
            .attached_shm(attached_shm)
            .vmo_offset(0)
            .handle_page_faults_around();
        options
    };
    // FIXME: Need to check whether current process has permission to access
    // the shared memory object.
    let map_addr = vm_map_options.build()?;

    Ok(SyscallReturn::Return(map_addr as _))
}
