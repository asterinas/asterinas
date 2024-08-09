// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_madvise(start: Vaddr, len: usize, behavior: i32) -> Result<SyscallReturn> {
    let behavior = MadviseBehavior::try_from(behavior)?;
    debug!(
        "start = 0x{:x}, len = 0x{:x}, behavior = {:?}",
        start, len, behavior
    );
    match behavior {
        MadviseBehavior::MADV_NORMAL
        | MadviseBehavior::MADV_SEQUENTIAL
        | MadviseBehavior::MADV_WILLNEED => {
            // perform a read at first
            let mut buffer = vec![0u8; len];
            CurrentUserSpace::get()
                .read_bytes(start, &mut VmWriter::from(buffer.as_mut_slice()))?;
        }
        MadviseBehavior::MADV_DONTNEED => {
            warn!("MADV_DONTNEED isn't implemented, do nothing for now.");
        }
        _ => todo!(),
    }
    Ok(SyscallReturn::Return(0))
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
/// This definition is the same from linux
pub enum MadviseBehavior {
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

    MADV_DONTDUMP = 16, /* Explicity exclude from the core dump,
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
