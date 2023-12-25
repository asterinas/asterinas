//! This module defines the UserVm of a process.
//! The UserSpace of a process only contains the virtual-physical memory mapping.
//! But we cannot know which vaddr is user heap, which vaddr is mmap areas.
//! So we define a UserVm struct to store such infomation.
//! Briefly, it contains the exact usage of each segment of virtual spaces.

pub mod user_heap;

use aster_rights::Full;
use user_heap::UserHeap;

use crate::vm::vmar::Vmar;

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
pub struct ProcessVm {
    user_heap: UserHeap,
    root_vmar: Vmar<Full>,
}

impl Clone for ProcessVm {
    fn clone(&self) -> Self {
        Self {
            root_vmar: self.root_vmar.dup().unwrap(),
            user_heap: self.user_heap.clone(),
        }
    }
}

impl ProcessVm {
    pub fn alloc() -> Self {
        let root_vmar = Vmar::<Full>::new_root();
        let user_heap = UserHeap::new();
        user_heap.init(&root_vmar);
        ProcessVm {
            user_heap,
            root_vmar,
        }
    }

    pub fn new(user_heap: UserHeap, root_vmar: Vmar<Full>) -> Self {
        Self {
            user_heap,
            root_vmar,
        }
    }

    pub fn user_heap(&self) -> &UserHeap {
        &self.user_heap
    }

    pub fn root_vmar(&self) -> &Vmar<Full> {
        &self.root_vmar
    }

    /// Set user vm to the init status
    pub fn clear(&self) {
        self.root_vmar.clear().unwrap();
        self.user_heap.set_default(&self.root_vmar);
    }
}
