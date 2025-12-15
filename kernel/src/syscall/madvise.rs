// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::vmar::VMAR_CAP_ADDR};

pub fn sys_madvise(addr: Vaddr, len: usize, behavior: i32, ctx: &Context) -> Result<SyscallReturn> {
    let behavior = MadviseBehavior::try_from(behavior)?;
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, behavior = {:?}",
        addr, len, behavior
    );

    if !addr.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "the mapping address is not aligned");
    }
    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if VMAR_CAP_ADDR.checked_sub(addr).is_none_or(|gap| gap < len) {
        // FIXME: Linux returns `EINVAL` if `(addr + len).align_up(PAGE_SIZE)` overflows. Here, we
        // perform a stricter validation.
        return_errno_with_message!(Errno::EINVAL, "the mapping range is not in userspace");
    }
    let addr_range = addr..(addr + len).align_up(PAGE_SIZE);

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();

    match behavior {
        MadviseBehavior::MADV_DONTNEED => {
            vmar.discard_pages(addr_range)?;
        }
        _ if DUMMY_MADVISE.contains(&behavior) => {
            let query_guard = vmar.query(addr_range);
            if !query_guard.is_fully_mapped() {
                return_errno_with_message!(
                    Errno::ENOMEM,
                    "the range contains pages that are not mapped"
                );
            }
            // For `DUMMY_MADVISE`, doing nothing is correct, though it may not be efficient.
        }
        _ => return_errno_with_message!(Errno::EINVAL, "the madvise behavior is not supported yet"),
    }

    Ok(SyscallReturn::Return(0))
}

// Reference: <https://elixir.bootlin.com/linux/v4.8/source/include/uapi/asm-generic/mman-common.h#L37>
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[expect(non_camel_case_types)]
enum MadviseBehavior {
    MADV_NORMAL = 0,     /* no further special treatment */
    MADV_RANDOM = 1,     /* expect random page references */
    MADV_SEQUENTIAL = 2, /* expect sequential page references */
    MADV_WILLNEED = 3,   /* will need these pages */
    MADV_DONTNEED = 4,   /* don't need these pages */

    /* common parameters: try to keep these consistent across architectures */
    MADV_FREE = 8,           /* free pages only if memory pressure */
    MADV_REMOVE = 9,         /* remove these pages & resources */
    MADV_DONTFORK = 10,      /* don't inherit across fork */
    MADV_DOFORK = 11,        /* do inherit across fork */
    MADV_HWPOISON = 100,     /* poison a page for testing */
    MADV_SOFT_OFFLINE = 101, /* soft offline page for testing */

    MADV_MERGEABLE = 12,   /* KSM may merge identical pages */
    MADV_UNMERGEABLE = 13, /* KSM may not merge identical pages */

    MADV_HUGEPAGE = 14,   /* Worth backing with hugepages */
    MADV_NOHUGEPAGE = 15, /* Not worth backing with hugepages */

    MADV_DONTDUMP = 16, /* Explicitly exclude from the core dump,
                        overrides the coredump filter bits */
    MADV_DODUMP = 17, /* Clear the MADV_DONTDUMP flag */

    MADV_WIPEONFORK = 18, /* Zero memory on fork, child only */
    MADV_KEEPONFORK = 19, /* Undo MADV_WIPEONFORK */

    MADV_COLD = 20,    /* deactivate these pages */
    MADV_PAGEOUT = 21, /* reclaim these pages */

    MADV_POPULATE_READ = 22,  /* populate (prefault) page tables readable */
    MADV_POPULATE_WRITE = 23, /* populate (prefault) page tables writable */

    MADV_DONTNEED_LOCKED = 24, /* like DONTNEED, but drop locked pages too */
}

/// Madvise that a dummy implementation is also correct.
///
/// This list can only contain madvise behaviors that do not alter the semantics of the user
/// program. In other words, they are intended solely for performance optimization and can safely
/// be ignored by the kernel.
///
/// **Please think twice before adding a new behavior to this list. Not all madvise behaviors can
/// be no-ops.**
const DUMMY_MADVISE: &[MadviseBehavior] = &[
    MadviseBehavior::MADV_NORMAL,
    MadviseBehavior::MADV_RANDOM,
    MadviseBehavior::MADV_SEQUENTIAL,
    MadviseBehavior::MADV_WILLNEED,
    MadviseBehavior::MADV_FREE,
    MadviseBehavior::MADV_MERGEABLE,
    MadviseBehavior::MADV_UNMERGEABLE,
    MadviseBehavior::MADV_HUGEPAGE,
    MadviseBehavior::MADV_NOHUGEPAGE,
];
