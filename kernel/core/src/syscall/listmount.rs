// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{fs::vfs::path::MNT_UNIQUE_ID_MIN, prelude::*};

/// Size in bytes of the first published `mnt_id_req` layout; the only
/// size currently accepted.
const MNT_ID_REQ_SIZE_VER0: u32 = 24;

/// Sentinel for `mnt_id_req::mnt_id` requesting the namespace root.
const LSMT_ROOT: u64 = u64::MAX;

/// `flags` bit: return descendants in descending `unique_id` order.
const LISTMOUNT_REVERSE: u32 = 1 << 0;

/// Upper bound on `nr_mnt_ids`.
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/namespace.c#L6036>
const LISTMOUNT_NR_LIMIT: usize = 1_000_000;

/// Linux's `struct mnt_id_req`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct MntIdReq {
    /// Caller-side `sizeof(struct mnt_id_req)`.
    size: u32,
    /// Reserved; must be zero.
    spare: u32,
    /// Parent mount's `unique_id`, or [`LSMT_ROOT`] for the namespace
    /// root.
    mnt_id: u64,
    /// Pagination cursor: only descendants with `unique_id > param`
    /// (or `<` in reverse) are returned. `0` starts from the beginning.
    param: u64,
}

const _: () = assert!(size_of::<MntIdReq>() as u32 == MNT_ID_REQ_SIZE_VER0);

pub fn sys_listmount(
    req_ptr: Vaddr,
    mnt_ids_ptr: Vaddr,
    nr_mnt_ids: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "req_ptr = 0x{:x}, mnt_ids_ptr = 0x{:x}, nr_mnt_ids = {}, flags = 0x{:x}",
        req_ptr, mnt_ids_ptr, nr_mnt_ids, flags,
    );

    if flags & !LISTMOUNT_REVERSE != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid listmount flags");
    }
    let reverse = flags & LISTMOUNT_REVERSE != 0;

    if nr_mnt_ids > LISTMOUNT_NR_LIMIT {
        return_errno_with_message!(Errno::EOVERFLOW, "nr_mnt_ids too large");
    }

    let user_space = ctx.user_space();

    // Validate the output buffer up front so an invalid pointer surfaces
    // as `EFAULT` before any namespace work is done.
    let mut mnt_ids_writer = user_space.writer(mnt_ids_ptr, nr_mnt_ids * size_of::<u64>())?;

    let req: MntIdReq = user_space.read_val(req_ptr)?;
    if req.size != MNT_ID_REQ_SIZE_VER0 {
        return_errno_with_message!(Errno::EINVAL, "unsupported mnt_id_req size");
    }
    if req.spare != 0 {
        return_errno_with_message!(Errno::EINVAL, "non-zero spare field");
    }
    if req.mnt_id != LSMT_ROOT && req.mnt_id < MNT_UNIQUE_ID_MIN {
        return_errno_with_message!(Errno::EINVAL, "mnt_id is not a valid unique mount ID");
    }
    if req.param != 0 && req.param < MNT_UNIQUE_ID_MIN {
        return_errno_with_message!(Errno::EINVAL, "param is not a valid unique mount ID");
    }

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let mnt_ns = ns_proxy.unwrap().mnt_ns();

    let parent_mount = if req.mnt_id == LSMT_ROOT {
        mnt_ns.root().clone()
    } else {
        mnt_ns
            .lookup_by_unique_id(req.mnt_id)
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "no such mount"))?
    };

    if nr_mnt_ids == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let mounts = mnt_ns.descendant_mounts_of(&parent_mount);
    let ids: Vec<_> = if reverse {
        mounts
            .rev()
            .map(|mount| mount.unique_id())
            .filter(|&id| req.param == 0 || id < req.param)
            .take(nr_mnt_ids)
            .collect()
    } else {
        mounts
            .map(|mount| mount.unique_id())
            .filter(|&id| id > req.param)
            .take(nr_mnt_ids)
            .collect()
    };

    for id in &ids {
        mnt_ids_writer.write_val(id)?;
    }

    Ok(SyscallReturn::Return(ids.len() as isize))
}
