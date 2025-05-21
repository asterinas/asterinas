// SPDX-License-Identifier: MPL-2.0

use core::mem::MaybeUninit;

use uefi::boot::AllocateType;

pub fn alloc_at(addr: usize, size: usize) -> &'static mut [MaybeUninit<u8>] {
    assert_ne!(addr, 0, "the address to allocate is zero");
    assert!(
        addr.checked_add(size).is_some(),
        "the range to allocate overflows"
    );

    let allocated = alloc_pages(AllocateType::Address(addr as u64), size);
    assert_eq!(
        allocated.as_ptr().addr(),
        addr,
        "the allocated address is not the request address"
    );

    allocated
}

pub(super) fn alloc_pages(ty: AllocateType, size: usize) -> &'static mut [MaybeUninit<u8>] {
    assert!(
        size <= isize::MAX as usize,
        "the size to allocate exceeds `isize::MAX`"
    );

    let allocated = uefi::boot::allocate_pages(
        ty,
        uefi::boot::MemoryType::LOADER_DATA,
        size.div_ceil(super::efi::PAGE_SIZE as usize),
    )
    .expect("the UEFI allocation fails");

    // SAFETY:
    // 1. The address is not zero and the size is reasonable (there are less the `isize::MAX` bytes
    //    and the range won't overflow the address space), as asserted above or guaranteed by the
    //    implementation of `allocate_pages`.
    // 2. The memory region is allocated via the UEFI firmware, so it is valid for reading and
    //    writing. We will not deallocate it, so it live for `'static`.
    // 3. The type alignment is 1 and the type can contain uninitialized data.
    unsafe { core::slice::from_raw_parts_mut(allocated.as_ptr().cast(), size) }
}
