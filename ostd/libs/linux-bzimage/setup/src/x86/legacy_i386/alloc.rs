// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use crate::sync::Mutex;

const NUM_USED_RANGES: usize = 16;

struct State {
    e820: &'static [linux_boot_params::BootE820Entry],
    used: [Range<usize>; NUM_USED_RANGES],
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// # Safety
///
/// The caller must ensure that the E820 entries correctly represent the current memory map.
pub(super) unsafe fn init(e820: &'static [linux_boot_params::BootE820Entry]) {
    let mut state = STATE.lock();

    assert!(state.is_none());

    extern "C" {
        fn __executable_start();
        fn __executable_end();
    }

    let mut used = core::array::from_fn(|_| 0..0);
    used[0] = (__executable_start as usize)..(__executable_end as usize);

    *state = Some(State { e820, used });
}

pub fn alloc_at(addr: usize, size: usize) -> &'static mut [u8] {
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

    state
        .e820
        .iter()
        .find(|entry| {
            let typ = entry.typ;
            typ == linux_boot_params::E820Type::Ram
                && entry.addr as usize <= range.start
                && (entry.addr + entry.size) as usize <= range.end
        })
        .expect("the range to allocate is not usable");

    let overlapped = state
        .used
        .iter()
        .find(|used| used.start < range.end && range.start < used.end);
    assert!(overlapped.is_none(), "the range to allocate is used");

    let empty = state
        .used
        .iter_mut()
        .find(|used| Range::is_empty(&used))
        .expect("the allocated ranges are full");
    *empty = range;

    // SAFETY:
    // 1. The address is not zero and the size is reasonable (there are less the `isize::MAX` bytes
    //    and the range won't overflow the address space), as asserted above.
    // 2. The memory region is usable and just allocated, so it is valid for reading and writing.
    //    We will not deallocate it, so it live for `'static`.
    // 3. Physical memory has been initialized by the firmware. The data type is plain-old-data and
    //    the type alignment is 1.
    unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, size) }
}
