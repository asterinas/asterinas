// SPDX-License-Identifier: MPL-2.0

use fdt::{node::FdtNode, standard_nodes::MemoryRegion};
use ostd::io::IoMem;

/// An extension trait that provides [`acquire_io_mems`] for [`FdtNode`].
///
/// [`acquire_io_mems`]: Self::acquire_io_mems
pub trait AcquireIoMems {
    /// Acquires `N` [`IoMem`]s according to the `reg` property of the device tree node.
    ///
    /// The caller is expected to pass an accurate value of `N`. This method will fail if the node
    /// contains too many or too few memory regions in the `reg` property.
    ///
    /// The caller is expected to pass the required size of each `IoMem` in `min_sizes`. This method
    /// will fail if a memory region in the `reg` property is smaller than the required size.
    ///
    /// This method will also fail if any memory region in the `reg` property is invalid or
    /// unavailable.
    fn acquire_io_mems<const N: usize>(&self, min_sizes: [usize; N]) -> Option<[IoMem; N]>;
}

impl AcquireIoMems for FdtNode<'_, '_> {
    fn acquire_io_mems<const N: usize>(&self, min_sizes: [usize; N]) -> Option<[IoMem; N]> {
        fn warn_count_does_not_match(count: usize, n: usize, name: &str) {
            ostd::warn!(
                "node '{}': expect {} MMIO regions, but found {} regions",
                name,
                n,
                count
            );
        }

        let Some(mut reg) = self.reg() else {
            warn_count_does_not_match(0, N, self.name);
            return None;
        };

        let next_fn = |i| -> Option<IoMem> {
            let Some(region) = reg.next() else {
                warn_count_does_not_match(i, N, self.name);
                return None;
            };
            acquire_io_mem(region, min_sizes[i], self.name)
        };
        let io_mems = core::array::try_from_fn(next_fn)?;

        let remain = reg.count();
        if remain != 0 {
            warn_count_does_not_match(N + remain, N, self.name);
            return None;
        }

        Some(io_mems)
    }
}

fn acquire_io_mem(region: MemoryRegion, min_size: usize, name: &str) -> Option<IoMem> {
    let addr = region.starting_address.addr();

    let Some(size) = region.size else {
        ostd::warn!("node '{}': MMIO region {:#x} lacks a size", name, addr);
        return None;
    };

    if size < min_size {
        ostd::warn!(
            "node '{}': MMIO region {:#x} is too small (size={:#x})",
            name,
            addr,
            size
        );
        return None;
    }

    let Some(addr_end) = addr.checked_add(size) else {
        ostd::warn!(
            "node '{}': MMIO region {:#x} overflows (size={:#x})",
            name,
            addr,
            size
        );
        return None;
    };

    let Ok(io_mem) = IoMem::acquire(addr..addr_end) else {
        ostd::warn!(
            "node '{}': MMIO region {:#x} is unavailable (size={:#x})",
            name,
            addr,
            size
        );
        return None;
    };

    Some(io_mem)
}
