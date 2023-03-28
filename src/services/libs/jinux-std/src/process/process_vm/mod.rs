//! This module defines the UserVm of a process.
//! The UserSpace of a process only contains the virtual-physical memory mapping.
//! But we cannot know which vaddr is user heap, which vaddr is mmap areas.
//! So we define a UserVm struct to store such infomation.
//! Briefly, it contains the exact usage of each segment of virtual spaces.

pub mod mmap_flags;
pub mod user_heap;

use crate::prelude::*;
use user_heap::UserHeap;

use crate::{rights::Full, vm::vmar::Vmar};

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
}

impl UserVm {
    pub fn new(root_vmar: &Vmar<Full>) -> Result<Self> {
        let user_heap = UserHeap::new();
        user_heap.init(root_vmar).unwrap();
        Ok(UserVm { user_heap })
    }

    pub fn user_heap(&self) -> &UserHeap {
        &self.user_heap
    }

    /// Set user vm to the init status
    pub fn set_default(&self) -> Result<()> {
        self.user_heap.set_default()
    }
}
