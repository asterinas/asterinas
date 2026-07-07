// SPDX-License-Identifier: MPL-2.0

//! Tracks System V semaphore adjustments recorded by `SEM_UNDO`.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    ipc::{IpcId, IpcNamespace},
    prelude::*,
    process::Pid,
};

/// Identifies one logical lifetime of a semaphore set for `SEM_UNDO`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemUndoToken {
    key: u64,
    epoch: u64,
}

impl SemUndoToken {
    /// Creates a semaphore undo token.
    pub fn new(key: u64, epoch: u64) -> Self {
        Self { key, epoch }
    }

    /// Returns whether `self` belongs to an earlier or same epoch of `other`.
    pub fn is_before_or_equal_to(self, other: Self) -> bool {
        self.key == other.key && self.epoch <= other.epoch
    }
}

/// A semaphore adjustment recorded for `SEM_UNDO`.
#[derive(Clone, Copy, Debug)]
pub struct SemUndoOp {
    sem_num: usize,
    adjustment: i32,
}

impl SemUndoOp {
    /// Creates a semaphore undo adjustment.
    pub fn new(sem_num: usize, adjustment: i32) -> Self {
        Self {
            sem_num,
            adjustment,
        }
    }
}

struct SemUndoEntry {
    ipc_ns: Arc<IpcNamespace>,
    sem_id: IpcId,
    token: SemUndoToken,
    sem_num: usize,
    adjustment: i32,
}

/// A System V semaphore undo list.
pub struct SemUndoList {
    entries: Mutex<Vec<SemUndoEntry>>,
    holders: AtomicUsize,
}

impl SemUndoList {
    /// Creates an empty semaphore undo list.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(Vec::new()),
            holders: AtomicUsize::new(1),
        })
    }

    /// Adds a process holder to the undo list.
    pub fn add_holder(&self) {
        self.holders.fetch_add(1, Ordering::Relaxed);
    }

    /// Removes a process holder and applies undo entries if it was the last one.
    pub fn remove_holder(&self, pid: Pid) {
        let old_holders = self.holders.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(old_holders > 0);
        if old_holders == 1 {
            self.apply(pid);
        }
    }

    /// Records successful `SEM_UNDO` operations.
    pub fn record(
        self: &Arc<Self>,
        ipc_ns: Arc<IpcNamespace>,
        sem_id: IpcId,
        token: SemUndoToken,
        undo_ops: &[SemUndoOp],
    ) {
        if undo_ops.is_empty() {
            return;
        }

        ipc_ns.register_sem_undo_list(self);

        let mut entries = self.entries.lock();
        if !ipc_ns.sem_undo_token_matches(sem_id, token) {
            return;
        }

        for op in undo_ops {
            if op.adjustment == 0 {
                continue;
            }

            let Some(index) = entries.iter().position(|entry| {
                Arc::ptr_eq(&entry.ipc_ns, &ipc_ns)
                    && entry.sem_id == sem_id
                    && entry.token == token
                    && entry.sem_num == op.sem_num
            }) else {
                entries.push(SemUndoEntry {
                    ipc_ns: ipc_ns.clone(),
                    sem_id,
                    token,
                    sem_num: op.sem_num,
                    adjustment: op.adjustment,
                });
                continue;
            };

            entries[index].adjustment += op.adjustment;
            if entries[index].adjustment == 0 {
                entries.remove(index);
            }
        }
    }

    /// Clears undo entries for one semaphore.
    pub fn clear_sem(
        &self,
        ipc_ns: &IpcNamespace,
        sem_id: IpcId,
        token: SemUndoToken,
        sem_num: usize,
    ) {
        self.entries.lock().retain(|entry| {
            !(core::ptr::eq(entry.ipc_ns.as_ref(), ipc_ns)
                && entry.sem_id == sem_id
                && entry.token.is_before_or_equal_to(token)
                && entry.sem_num == sem_num)
        });
    }

    /// Clears undo entries for all semaphores in a set.
    pub fn clear_set(&self, ipc_ns: &IpcNamespace, sem_id: IpcId, token: SemUndoToken) {
        self.entries.lock().retain(|entry| {
            !(core::ptr::eq(entry.ipc_ns.as_ref(), ipc_ns)
                && entry.sem_id == sem_id
                && entry.token.is_before_or_equal_to(token))
        });
    }

    /// Applies and clears all undo entries.
    fn apply(&self, pid: Pid) {
        let entries = core::mem::take(&mut *self.entries.lock());

        for entry in entries {
            entry.ipc_ns.apply_sem_undo(
                entry.sem_id,
                entry.token,
                entry.sem_num,
                entry.adjustment,
                pid,
            );
        }
    }
}
