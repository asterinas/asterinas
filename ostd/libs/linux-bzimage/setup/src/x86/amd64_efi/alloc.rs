// SPDX-License-Identifier: MPL-2.0

pub fn alloc_at(addr: usize, size: usize) -> &'static mut [u8] {
    assert_ne!(addr, 0, "the address to allocate is zero");
    assert!(
        size <= isize::MAX as usize,
        "the size to allocate exceeds `isize::MAX`"
    );

    addr.checked_add(size)
        .expect("the range to allocate overflows");

    let allocated = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::Address(addr as u64),
        uefi::boot::MemoryType::LOADER_DATA,
        size.div_ceil(super::efi::PAGE_SIZE as usize),
    )
    .expect("the UEFI allocation fails");
    assert_eq!(
        allocated.as_ptr() as usize,
        addr,
        "the allocated address is not the request address"
    );

    // SAFETY:
    // 1. The address is not zero and the size is reasonable (there are less the `isize::MAX` bytes
    //    and the range won't overflow the address space), as asserted above.
    // 2. The memory region is allocated via the UEFI firmware, so it is valid for reading and
    //    writing. We will not deallocate it, so it live for `'static`.
    // 3. Physical memory has been initialized by the firmware. The data type is plain-old-data and
    //    the type alignment is 1.
    unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, size) }
}
