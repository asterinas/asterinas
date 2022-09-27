//! This module defines the UserVm of a process.
//! The UserSpace of a process only contains the virtual-physical memory mapping.
//! But we cannot know which vaddr is user heap, which vaddr is mmap areas.
//! So we define a UserVm struct to store such infomation.
//! Briefly, it contains the exact usage of each segment of virtual spaces.

use crate::memory::{mmap_area::MmapArea, user_heap::UserHeap};

/*
* The user vm space layout is look like below.
* |-----------------------|-------The highest user vm address
* |                       |
* |       Mmap Areas      |
* |                       |
* |                       |
* --------------------------------The init stack base
* |                       |
* | User Stack(Init Stack)|
* |                       |
* |         ||            |
* ----------||----------------------The user stack top, grows down
* |         \/            |
* |                       |
* |     Unmapped Areas    |
* |                       |
* |         /\            |
* ----------||---------------------The user heap top, grows up
* |         ||            |
* |                       |
* |        User Heap      |
* |                       |
* ----------------------------------The user heap base
*/

/// The virtual space usage.
/// This struct is used to control brk and mmap now.
#[derive(Debug, Clone)]
pub struct UserVm {
    user_heap: UserHeap,
    mmap_area: MmapArea,
}

impl UserVm {
    pub const fn new() -> Self {
        let user_heap = UserHeap::new();
        let mmap_area = MmapArea::new();
        UserVm {
            user_heap,
            mmap_area,
        }
    }

    pub fn user_heap(&self) -> &UserHeap {
        &self.user_heap
    }

    pub fn mmap_area(&self) -> &MmapArea {
        &self.mmap_area
    }
}
