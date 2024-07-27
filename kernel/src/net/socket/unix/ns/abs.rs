// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::Entry, format};

use keyable_arc::KeyableArc;

use crate::prelude::*;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AbstractHandle(KeyableArc<[u8]>);

impl AbstractHandle {
    fn new(name: Arc<[u8]>) -> Self {
        Self(KeyableArc::from(name))
    }

    pub fn name(&self) -> Arc<[u8]> {
        self.0.clone().into()
    }
}

impl Drop for AbstractHandle {
    fn drop(&mut self) {
        HANDLE_TABLE.remove(self.name());
    }
}

static HANDLE_TABLE: HandleTable = HandleTable::new();

struct HandleTable {
    handles: RwLock<BTreeMap<Arc<[u8]>, Weak<AbstractHandle>>>,
}

impl HandleTable {
    const fn new() -> Self {
        Self {
            handles: RwLock::new(BTreeMap::new()),
        }
    }

    fn create(&self, name: Arc<[u8]>) -> Option<Arc<AbstractHandle>> {
        let mut handles = self.handles.write();

        let mut entry = handles.entry(name.clone());

        if let Entry::Occupied(ref occupied) = entry {
            // The handle is in use only if its strong count is greater than zero.
            if occupied.get().strong_count() > 0 {
                return None;
            }
        }

        let new_handle = Arc::new(AbstractHandle::new(name));
        let weak_handle = Arc::downgrade(&new_handle);

        match entry {
            Entry::Occupied(ref mut occupied) => {
                occupied.insert(weak_handle);
            }
            Entry::Vacant(vacant) => {
                vacant.insert(weak_handle);
            }
        }

        Some(new_handle)
    }

    fn remove(&self, name: Arc<[u8]>) {
        let mut handles = self.handles.write();

        let Entry::Occupied(occupied) = handles.entry(name) else {
            return;
        };

        // Due to race conditions between `AbstractHandle::drop` and `HandleTable::create`, the
        // entry may be occupied by another handle.
        //
        // Therefore, before removing the entry, we must check again if the entry should be removed.
        if occupied.get().strong_count() == 0 {
            occupied.remove();
        }
    }

    fn lookup(&self, name: &[u8]) -> Option<Arc<AbstractHandle>> {
        let handles = self.handles.read();

        handles.get(name).and_then(Weak::upgrade)
    }

    fn alloc_ephemeral(&self) -> Option<Arc<AbstractHandle>> {
        // See "Autobind feature" in the man pages:
        // <https://man7.org/linux/man-pages/man7/unix.7.html>.
        //
        // Note that false negatives are fine here. So we don't mind race conditions.
        //
        // TODO: Always starting with the first name is inefficient and leads to contention.
        // Instead, we should generate some random names and check their availability.
        (0..(1 << 20))
            .map(|num| format!("{:05x}", num))
            .map(|name| Arc::from(name.as_bytes()))
            .filter_map(|name| self.create(name))
            .next()
    }
}

pub fn create_abstract_name(name: Arc<[u8]>) -> Result<Arc<AbstractHandle>> {
    HANDLE_TABLE.create(name).ok_or_else(|| {
        Error::with_message(Errno::EADDRINUSE, "the abstract name is already in use")
    })
}

pub fn alloc_ephemeral_abstract_name() -> Result<Arc<AbstractHandle>> {
    HANDLE_TABLE.alloc_ephemeral().ok_or_else(|| {
        Error::with_message(Errno::ENOSPC, "no ephemeral abstract name is available")
    })
}

pub fn lookup_abstract_name(name: &[u8]) -> Result<Arc<AbstractHandle>> {
    HANDLE_TABLE
        .lookup(name)
        .ok_or_else(|| Error::with_message(Errno::ECONNREFUSED, "the abstract name does not exist"))
}
