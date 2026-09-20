// SPDX-License-Identifier: MPL-2.0

//! Device memory objects and pending mmap operations.
//!
//! Drivers prepare physical memory before the VMAR takes page-table locks.
//! Until publication, the operations remain in the DMO so driver invalidation
//! can remove pages from both pending and installed mappings. Taking pending
//! operations and publishing their reverse mappings hold the same rmap lock.
//! Device mappings are eager; missing/revoked pages have no refill callback yet.
//! Private mappings are read-only until device copy-on-write is implemented.

use core::ops::Range;

use ostd::{
    io::IoMem,
    mm::{HasPaddr, HasSize, UFrame},
};

use crate::{prelude::*, vm::vmar::Rmap};

/// A device that prepares mmap operations without holding page-table locks.
pub(crate) trait DeviceMappable: Send + Sync + Debug {
    /// Prepares operations at device offsets, without knowing their future VAs.
    ///
    /// Drivers must serialize preparing/registering operations and invalidating
    /// them with their driver lock. The returned handle owns the registration;
    /// dropping it cancels the operation, including on mmap failure.
    fn prepare_mapping(&self, range: Range<usize>) -> Result<PendingDmoMapping>;
}

/// Physical memory to map at an offset in the device object, not a virtual address.
#[derive(Debug)]
pub(crate) enum MapOperation {
    /// A base-page RAM frame and its device offset.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "for drivers mapping RAM, such as GPU UVM")
    )]
    Frame(UFrame, usize),
    /// A page-aligned MMIO region and its device offset.
    IoMem(IoMem, usize),
}

impl MapOperation {
    pub(super) fn range(&self) -> Range<usize> {
        match self {
            Self::Frame(_, offset) => *offset..*offset + PAGE_SIZE,
            Self::IoMem(memory, offset) => *offset..*offset + memory.size(),
        }
    }

    fn checked_range(&self) -> Result<Range<usize>> {
        let (offset, size) = match self {
            Self::Frame(frame, offset) if frame.map_level() == 1 => (*offset, PAGE_SIZE),
            Self::IoMem(memory, offset)
                if memory.paddr().is_multiple_of(PAGE_SIZE)
                    && memory.size().is_multiple_of(PAGE_SIZE) =>
            {
                (*offset, memory.size())
            }
            _ => return_errno_with_message!(Errno::EINVAL, "device memory is not page aligned"),
        };
        if !offset.is_multiple_of(PAGE_SIZE) || size == 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid device mapping offset or size");
        }
        let end = offset.checked_add(size).ok_or(Errno::EINVAL)?;
        Ok(offset..end)
    }

    fn exclude(self, range: &Range<usize>, output: &mut Vec<Self>) {
        let own = self.range();
        if own.end <= range.start || range.end <= own.start {
            output.push(self);
            return;
        }
        // Frame operations are base pages, so an aligned overlap removes the
        // whole frame. MMIO operations may straddle either invalidation edge.
        if let Self::IoMem(memory, offset) = self {
            if own.start < range.start {
                output.push(Self::IoMem(memory.slice(0..range.start - offset), offset));
            }
            if range.end < own.end {
                output.push(Self::IoMem(
                    memory.slice(range.end - offset..memory.size()),
                    range.end,
                ));
            }
        }
    }
}

/// A device memory object, shared by all mappings of one driver-managed object.
#[derive(Debug)]
pub(crate) struct Dmo {
    pending: SpinLock<PendingOperations>,
    rmap: Mutex<Rmap>,
}

#[derive(Debug)]
struct PendingOperations {
    next_id: usize,
    entries: BTreeMap<usize, Vec<MapOperation>>,
}

impl Dmo {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            pending: SpinLock::new(PendingOperations {
                next_id: 0,
                entries: BTreeMap::new(),
            }),
            rmap: Mutex::new(Rmap::new()),
        })
    }

    /// Registers a prepared mapping while the caller still holds its driver lock.
    ///
    /// Operations must be nonoverlapping and lie within the requested range.
    /// Holes are permitted; accesses to unpopulated device pages fail until a
    /// driver provides a page-fault implementation (not part of this mmap API).
    pub(crate) fn prepare(
        self: &Arc<Self>,
        range: Range<usize>,
        mut operations: Vec<MapOperation>,
    ) -> Result<PendingDmoMapping> {
        check_range(&range)?;
        for operation in &operations {
            let op_range = operation.checked_range()?;
            if op_range.start < range.start || op_range.end > range.end {
                return_errno_with_message!(Errno::EINVAL, "device operation exceeds mapping range");
            }
        }
        operations.sort_unstable_by_key(|operation| operation.range().start);
        if operations
            .windows(2)
            .any(|pair| pair[0].range().end > pair[1].range().start)
        {
            return_errno_with_message!(Errno::EINVAL, "device mapping operations overlap");
        }

        let mut pending = self.pending.lock();
        let id = pending.next_id;
        pending.next_id = id.checked_add(1).ok_or(Errno::EOVERFLOW)?;
        pending.entries.insert(id, operations);
        Ok(PendingDmoMapping {
            dmo: self.clone(),
            id,
            range,
        })
    }

    /// Removes a device-offset range from pending and installed mappings.
    ///
    /// The driver must hold the same lock it uses for preparing/registering new
    /// operations, and must prevent refaults from reintroducing these pages.
    /// Call outside atomic mode, before migrating/reusing the removed memory.
    #[cfg_attr(
        not(ktest),
        expect(dead_code, reason = "for drivers that revoke or migrate mapped memory")
    )]
    pub(crate) fn unmap(&self, range: Range<usize>) -> Result<()> {
        check_range(&range)?;
        let mut rmap = self.rmap.lock();
        {
            let mut pending = self.pending.lock();
            for operations in pending.entries.values_mut() {
                let old = core::mem::take(operations);
                for operation in old {
                    operation.exclude(&range, operations);
                }
            }
        }
        rmap.unmap(range);
        Ok(())
    }

    pub(in crate::vm) fn rmap(&self) -> &Mutex<Rmap> {
        &self.rmap
    }
}

/// An owned pending mapping; dropping it cancels any unpublished operations.
#[derive(Debug)]
pub(crate) struct PendingDmoMapping {
    dmo: Arc<Dmo>,
    id: usize,
    range: Range<usize>,
}

impl PendingDmoMapping {
    pub(in crate::vm) fn dmo(&self) -> &Arc<Dmo> {
        &self.dmo
    }

    pub(in crate::vm) fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// Takes the remaining operations while serializing with invalidation.
    pub(in crate::vm) fn take(self, rmap: &mut MutexGuard<'_, Rmap>) -> Vec<MapOperation> {
        assert!(core::ptr::eq(MutexGuard::get_lock(rmap), &self.dmo.rmap));
        let mut pending = self.dmo.pending.lock();
        let operations = pending.entries.remove(&self.id).unwrap();
        // The handle's destructor also takes this lock to cancel registrations.
        drop(pending);
        operations
    }
}

impl Drop for PendingDmoMapping {
    fn drop(&mut self) {
        self.dmo.pending.lock().entries.remove(&self.id);
    }
}

fn check_range(range: &Range<usize>) -> Result<()> {
    if range.start >= range.end
        || !range.start.is_multiple_of(PAGE_SIZE)
        || !range.end.is_multiple_of(PAGE_SIZE)
    {
        return_errno_with_message!(Errno::EINVAL, "invalid device mapping range");
    }
    Ok(())
}

#[cfg(ktest)]
mod tests {
    use ostd::{mm::FrameAllocOptions, prelude::ktest};

    use super::*;

    #[ktest]
    fn pending_handles_cancel_on_drop() {
        let dmo = Dmo::new();
        let frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let pending = dmo
            .prepare(
                0..PAGE_SIZE,
                vec![MapOperation::Frame(frame.clone().into(), 0)],
            )
            .unwrap();
        assert_eq!(dmo.pending.lock().entries.len(), 1);
        drop(pending);
        assert!(dmo.pending.lock().entries.is_empty());
        assert_eq!(frame.reference_count(), 1);
    }

    #[ktest]
    fn invalidation_removes_pending_frames_and_splits_mmio() {
        const IO_BASE: usize = 0x200_0000_0000;
        let dmo = Dmo::new();
        let frame = FrameAllocOptions::new().alloc_frame().unwrap();
        let memory = IoMem::acquire(IO_BASE..IO_BASE + 3 * PAGE_SIZE).unwrap();
        let pending_ram = dmo
            .prepare(0..PAGE_SIZE, vec![MapOperation::Frame(frame.into(), 0)])
            .unwrap();
        let pending_io = dmo
            .prepare(0..3 * PAGE_SIZE, vec![MapOperation::IoMem(memory, 0)])
            .unwrap();

        dmo.unmap(PAGE_SIZE..2 * PAGE_SIZE).unwrap();
        let mut rmap = dmo.rmap.lock();
        let operations = pending_io.take(&mut rmap);
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0].range(), 0..PAGE_SIZE);
        assert_eq!(operations[1].range(), 2 * PAGE_SIZE..3 * PAGE_SIZE);
        let MapOperation::IoMem(right, _) = &operations[1] else {
            panic!("expected MMIO");
        };
        assert_eq!(right.paddr(), IO_BASE + 2 * PAGE_SIZE);
        drop(rmap);

        dmo.unmap(0..PAGE_SIZE).unwrap();
        assert!(pending_ram.take(&mut dmo.rmap.lock()).is_empty());
        assert!(dmo.pending.lock().entries.is_empty());
    }

    #[ktest]
    fn invalid_operations_are_not_registered() {
        let dmo = Dmo::new();
        let frame: UFrame = FrameAllocOptions::new().alloc_frame().unwrap().into();
        assert!(
            dmo.prepare(
                0..PAGE_SIZE,
                vec![MapOperation::Frame(frame.clone(), PAGE_SIZE)]
            )
            .is_err()
        );
        assert!(
            dmo.prepare(
                0..PAGE_SIZE,
                vec![
                    MapOperation::Frame(frame.clone(), 0),
                    MapOperation::Frame(frame.clone(), 0)
                ]
            )
            .is_err()
        );
        assert!(
            dmo.prepare(0..PAGE_SIZE, vec![MapOperation::Frame(frame, 1)])
                .is_err()
        );
        assert!(dmo.prepare(0..0, Vec::new()).is_err());
        assert!(dmo.pending.lock().entries.is_empty());
    }
}
