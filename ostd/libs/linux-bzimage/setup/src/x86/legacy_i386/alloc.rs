// SPDX-License-Identifier: MPL-2.0

use core::{mem::MaybeUninit, ops::Range};

use crate::sync::Mutex;

const NUM_USED_RANGES: usize = 16;

struct State {
    e820: &'static [linux_boot_params::BootE820Entry],
    used: [Range<usize>; NUM_USED_RANGES],
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// # Safety
///
/// The caller must ensure that the E820 entries in the boot parameters correctly represent the
/// current memory map.
pub(super) unsafe fn init(boot_params: &'static linux_boot_params::BootParams) {
    let mut state = STATE.lock();

    assert!(state.is_none());

    let mut used = core::array::from_fn(|_| 0..0);

    extern "C" {
        fn __executable_start();
        fn __executable_end();
    }

    used[0] = (__executable_start as usize)..(__executable_end as usize);

    fn range_from_start_and_len(start: usize, len: usize) -> Range<usize> {
        start..start.checked_add(len).unwrap()
    }

    used[1] = range_from_start_and_len(
        core::ptr::from_ref(boot_params).addr(),
        core::mem::size_of::<linux_boot_params::BootParams>(),
    );
    // No need to worry about `ext_*` addresses/sizes since we're 32-bit.
    used[2] = range_from_start_and_len(
        boot_params.hdr.cmd_line_ptr as usize,
        boot_params.hdr.cmdline_size as usize,
    );
    used[3] = range_from_start_and_len(
        boot_params.hdr.ramdisk_image as usize,
        boot_params.hdr.ramdisk_size as usize,
    );

    *state = Some(State {
        e820: &boot_params.e820_table[..(boot_params.e820_entries as usize)],
        used,
    });
}

pub fn alloc_at(addr: usize, size: usize) -> &'static mut [MaybeUninit<u8>] {
    let mut state = STATE.lock();
    let state = state.as_mut().unwrap();

    assert_ne!(addr, 0, "the address to allocate is zero");
    assert!(
        size <= isize::MAX as usize,
        "the size to allocate exceeds `isize::MAX`"
    );

    let range = addr..addr
        .checked_add(size)
        .expect("the range to allocate overflows");

    assert!(
        state.e820.iter().any(|entry| {
            let typ = entry.typ;
            typ == linux_boot_params::E820Type::Ram
                && entry.addr as usize <= range.start
                && range.end <= (entry.addr + entry.size) as usize
        }),
        "the range to allocate is not usable"
    );

    let overlapped = state
        .used
        .iter()
        .find(|used| used.start < range.end && range.start < used.end);
    assert!(overlapped.is_none(), "the range to allocate is used");

    let empty = state
        .used
        .iter_mut()
        // Use fully qualified syntax to avoid collisions with the unstable method
        // `ExactSizeIterator::is_empty`. See <https://github.com/rust-lang/rust/issues/86682>.
        .find(|used| Range::is_empty(used))
        .expect("the allocated ranges are full");
    *empty = range;

    // SAFETY:
    // 1. The address is not zero and the size is reasonable (there are less the `isize::MAX` bytes
    //    and the range won't overflow the address space), as asserted above.
    // 2. The memory region is usable and just allocated, so it is valid for reading and writing.
    //    We will not deallocate it, so it live for `'static`.
    // 3. The type alignment is 1 and the type can contain uninitialized data.
    unsafe { core::slice::from_raw_parts_mut(addr as *mut MaybeUninit<u8>, size) }
}
