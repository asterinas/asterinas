// SPDX-License-Identifier: MPL-2.0

//! The implementation of robust list is from occlum.

use ostd::task::Task;

use crate::{current_userspace, prelude::*, process::posix_thread::futex::futex_wake, thread::Tid};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct RobustList {
    next: Vaddr, // *const Robust list
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct RobustListHead {
    /// Linked list of lock entries
    ///
    /// If it points to the head of the list, then it is the end of the list.
    /// If it is an invalid user space pointer or a null pointer, stop iterating
    /// the list.
    list: RobustList,
    /// Specifies the offset from the address of the lock entry to the address
    /// of the futex.
    futex_offset: isize,
    /// Contains transient copy of the address of the lock entry, during list
    /// insertion and removal.
    list_op_pending: Vaddr, // *const RobustList
}

impl RobustListHead {
    /// Return an iterator for all futexes in the robust list.
    ///
    /// The futex referred to by `list_op_pending`, if any, will be returned as
    /// the last item.
    pub fn futexes(&self) -> FutexIter<'_> {
        FutexIter::new(self)
    }

    /// Return the pending futex address if exist
    fn pending_futex_addr(&self) -> Option<Vaddr> {
        if self.list_op_pending == 0 {
            None
        } else {
            Some(self.futex_addr(self.list_op_pending))
        }
    }

    /// Get the futex address
    fn futex_addr(&self, entry_ptr: Vaddr) -> Vaddr {
        (entry_ptr as isize + self.futex_offset) as _
    }
}

pub struct FutexIter<'a> {
    robust_list: &'a RobustListHead,
    entry_ptr: Vaddr,
    count: isize,
}

impl<'a> FutexIter<'a> {
    pub fn new(robust_list: &'a RobustListHead) -> Self {
        Self {
            robust_list,
            entry_ptr: robust_list.list.next,
            count: 0,
        }
    }

    // The `self.count` is normally a positive value used to iterate the list
    // to avoid excessively long or circular list, we use a special value -1
    // to represent the end of the Iterator.
    fn set_end(&mut self) {
        self.count = -1;
    }

    fn is_end(&self) -> bool {
        self.count < 0
    }
}

const ROBUST_LIST_LIMIT: isize = 2048;

impl Iterator for FutexIter<'_> {
    type Item = Vaddr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_end() {
            return None;
        }

        let end_ptr = self.robust_list.list.next;
        while self.entry_ptr != end_ptr || self.count == 0 {
            if self.count == ROBUST_LIST_LIMIT {
                break;
            }
            if self.entry_ptr == 0 {
                return None;
            }
            let futex_addr = if self.entry_ptr != self.robust_list.list_op_pending {
                Some(self.robust_list.futex_addr(self.entry_ptr))
            } else {
                None
            };
            let Ok(robust_list) = current_userspace!().read_val::<RobustList>(self.entry_ptr)
            else {
                return None;
            };
            self.entry_ptr = robust_list.next;
            self.count += 1;
            if futex_addr.is_some() {
                return futex_addr;
            }
        }
        self.set_end();
        self.robust_list.pending_futex_addr()
    }
}

const FUTEX_WAITERS: u32 = 0x8000_0000;
const FUTEX_OWNER_DIED: u32 = 0x4000_0000;
const FUTEX_TID_MASK: u32 = 0x3FFF_FFFF;

/// Wakeup one robust futex owned by the thread
/// FIXME: requires atomic operations here
pub fn wake_robust_futex(futex_addr: Vaddr, tid: Tid) -> Result<()> {
    let task = Task::current().unwrap();
    let user_space = CurrentUserSpace::new(&task);

    let futex_val = {
        if futex_addr == 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid futext addr");
        }
        user_space.read_val::<u32>(futex_addr)?
    };
    let mut old_val = futex_val;
    loop {
        // This futex may held by another thread, do nothing
        if old_val & FUTEX_TID_MASK != tid {
            break;
        }
        let new_val = (old_val & FUTEX_WAITERS) | FUTEX_OWNER_DIED;
        let cur_val = user_space.read_val(futex_addr)?;
        if cur_val != new_val {
            // The futex value has changed, let's retry with current value
            old_val = cur_val;
            user_space.write_val(futex_addr, &new_val)?;
            continue;
        }
        // Wakeup one waiter
        if cur_val & FUTEX_WAITERS != 0 {
            debug!("wake robust futex addr: {:?}", futex_addr);
            futex_wake(futex_addr, 1, None)?;
        }
        break;
    }
    Ok(())
}
