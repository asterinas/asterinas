// SPDX-License-Identifier: MPL-2.0

//! The implementation of robust list is from occlum.

use ostd::{mm::io::AtomicUpdateCurVal, task::Task};

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
            self.futex_addr(self.list_op_pending)
        }
    }

    /// Get the futex address
    fn futex_addr(&self, entry_ptr: Vaddr) -> Option<Vaddr> {
        self.futex_offset
            .checked_add(entry_ptr as isize)
            .map(|result| result as Vaddr)
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
                self.robust_list.futex_addr(self.entry_ptr)
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

/// Attempts to release and wake a robust futex owned by the given thread.
///
/// If the futex at `futex_addr` is still owned by `tid`, it is marked with
/// `FUTEX_OWNER_DIED` and one waiter (if any) is woken.  
/// If the futex is owned by another thread, the operation is canceled.
///
/// Errors if the address is invalid or not in user space.
pub fn wake_robust_futex(futex_addr: Vaddr, tid: Tid) -> Result<()> {
    if futex_addr == 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid futext address");
    }

    let task = Task::current().unwrap();
    let user_space = CurrentUserSpace::new(task.as_thread_local().unwrap());

    // Instantiate reader and writer pointing at the same `futex_addr` and set
    // up for the same length: the length of an `u32`.
    const U32_LEN: usize = size_of::<u32>();
    let reader = user_space.reader_checked(futex_addr, U32_LEN)?;
    let mut writer = user_space.writer(futex_addr, U32_LEN)?;

    // Only provide the next value for update if the futex is owned by `tid`.
    // Otherwise, cancel the update.
    let new_val_op = |val: u32| {
        if val & FUTEX_TID_MASK != tid {
            // This futex is held by another thread. Cancel update.
            None
        } else {
            Some((val & FUTEX_WAITERS) | FUTEX_OWNER_DIED)
        }
    };

    loop {
        let atomic_update_result = writer.atomic_update(&reader, new_val_op)?;
        match atomic_update_result.cur_val {
            AtomicUpdateCurVal::Canceled => break, // Futex held by another thread.
            AtomicUpdateCurVal::Succeeded(cur_val) => {
                // Wakeup one waiter and break out from the loop.
                if cur_val & FUTEX_WAITERS != 0 {
                    debug!("wake robust futex addr: {:?}", futex_addr);
                    futex_wake(futex_addr, 1, None)?;
                }
                break;
            }
            AtomicUpdateCurVal::Failed(_) => continue, // Try again.
        }
    }
    Ok(())
}
