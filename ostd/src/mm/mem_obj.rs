// SPDX-License-Identifier: MPL-2.0

//! Traits for memory objects.

use alloc::{boxed::Box, rc::Rc, sync::Arc};
use core::ops::Range;

use super::Paddr;
use crate::mm::Daddr;

/// Memory objects that have a start physical address.
pub trait HasPaddr {
    /// Returns the start physical address of the memory object.
    fn paddr(&self) -> Paddr;
}

/// Memory objects that have a mapped address in the device address space.
pub trait HasDaddr {
    /// Returns the base address of the mapping in the device address space.
    fn daddr(&self) -> Daddr;
}

/// Memory objects that have a length in bytes.
pub trait HasSize {
    /// Returns the size of the memory object in bytes.
    fn size(&self) -> usize;
}

/// Memory objects that have a physical address range.
pub trait HasPaddrRange: HasPaddr + HasSize {
    /// Returns the end physical address of the memory object.
    fn end_paddr(&self) -> Paddr;

    /// Returns the physical address range of the memory object.
    fn paddr_range(&self) -> Range<Paddr>;
}

impl<T: HasPaddr + HasSize> HasPaddrRange for T {
    fn end_paddr(&self) -> Paddr {
        self.paddr() + self.size()
    }

    fn paddr_range(&self) -> Range<Paddr> {
        self.paddr()..self.end_paddr()
    }
}

macro_rules! impl_has_traits_for_ref_type {
    ($t:ty, $([$trait_name:ident, $fn_name:ident]),*) => {
        $(
            impl<T: $trait_name> $trait_name for $t {
                fn $fn_name(&self) -> usize {
                    (**self).$fn_name()
                }
            }
        )*
    };
    ($($t:ty),*) => {
        $(
            impl_has_traits_for_ref_type!($t, [HasPaddr, paddr], [HasDaddr, daddr], [HasSize, size]);
        )*
    };
}

impl_has_traits_for_ref_type!(&T, &mut T, Rc<T>, Arc<T>, Box<T>);

/// Memory objects that can be split into smaller parts.
pub trait Split: Sized + HasSize {
    /// Splits the memory object into two at the given byte offset from the
    /// start.
    ///
    /// The resulting memory object cannot be empty. So the offset cannot be
    /// neither zero nor the length of the memory object.
    ///
    /// # Panics
    ///
    /// The function panics if the offset is out of bounds, at either ends, or
    /// not base-page-aligned.
    fn split(self, offset: usize) -> (Self, Self);
}
