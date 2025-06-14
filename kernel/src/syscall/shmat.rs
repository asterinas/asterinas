// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmat

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    fs::utils::InodeMode,
    prelude::*,
    syscall::mmap::{do_sys_mmap, MMapFlags, MMapOptions, MMapType, MmapHandle},
    vm::{
        perms::VmPerms,
        shared_mem::{SHMLBA, SHM_OBJ_MANAGER},
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
    if !(0..=0o177777).contains(&flags) {
        return_errno!(Errno::EINVAL);
    }

    let manager = SHM_OBJ_MANAGER.get().ok_or(Errno::EINVAL)?;
    let shm_obj = match manager.get_shm_obj(shmid as u64) {
        Some(shm_obj) => shm_obj,
        None => return_errno!(Errno::EINVAL),
    };

    let shm_flags = ShmFlags::from_bits_truncate(flags as u32);
    let mut perms = VmPerms::empty();
    if shm_flags.contains(ShmFlags::RDONLY) {
        if !shm_obj.mode()?.contains(InodeMode::S_IRUSR) {
            return_errno!(Errno::EACCES);
        }
        perms |= VmPerms::READ;
    } else {
        perms |= VmPerms::READ | VmPerms::WRITE;
    }
    if shm_flags.contains(ShmFlags::EXEC) {
        if !shm_obj.mode()?.contains(InodeMode::S_IXUSR) {
            return_errno!(Errno::EACCES);
        }
        perms |= VmPerms::EXEC;
    }

    let addr = if addr == 0 {
        // If addr is 0, the system chooses the address
        0_usize
    } else if shm_flags.contains(ShmFlags::RND) {
        // If RND is set, align down the address to SHMLBA
        addr.align_down(SHMLBA as u64) as usize
    } else if addr % SHMLBA as u64 != 0 {
        // If the address is not aligned with SHMLBA, return error
        return_errno!(Errno::EINVAL);
    } else {
        // Otherwise, use the provided address
        addr as usize
    };

    // Convert shmflg to MMapOptions
    let mut map_flags = MMapFlags::empty();
    if shm_flags.contains(ShmFlags::REMAP) {
        map_flags |= MMapFlags::MAP_FIXED;
    }

    let option = MMapOptions::try_from(map_flags.bits() | (MMapType::Shared as u32))?;

    // FIXME: Need to check whether current process has permission to access the shared memory object.
    shm_obj.set_attached(ctx.process.pid());
    let res = do_sys_mmap(
        addr,
        shm_obj.size(),
        perms,
        option,
        MmapHandle::Shared(shm_obj.shmid()),
        0, // offset is always 0 for shared memory
        ctx,
    )?;

    Ok(SyscallReturn::Return(res as _))
}
