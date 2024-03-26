// SPDX-License-Identifier: MPL-2.0

//! This module defines struct `ProcessVm`
//! to represent the layout of user space process virtual memory.
//!
//! The `ProcessVm` struct contains `Vmar`,
//! which stores all existing memory mappings.
//! The `Vm` also contains
//! the basic info of process level vm segments,
//! like init stack and heap.

mod heap;
mod stack;

use aster_rights::Full;
pub use heap::Heap;
pub use stack::Stack;

use crate::vm::vmar::Vmar;

/*
 * The user's virtual memory space layout looks like below.
 * TODO: The layout of the userheap does not match the current implementation,
 * And currently the initial program break is a fixed value.
 *
 *  (high address)
 *  +---------------------+ <------+ The top of Vmar, which is the highest address usable
 *  |                     |          Randomly padded pages
 *  +---------------------+ <------+ The base of the initial user stack
 *  | User stack          |
 *  |                     |
 *  +---------||----------+ <------+ The user stack limit, can be extended lower
 *  |         \/          |
 *  | ...                 |
 *  |                     |
 *  | MMAP Spaces         |
 *  |                     |
 *  | ...                 |
 *  |         /\          |
 *  +---------||----------+ <------+ The current program break
 *  | User heap           |
 *  |                     |
 *  +---------------------+ <------+ The original program break
 *  |                     |          Randomly padded pages
 *  +---------------------+ <------+ The end of the program's last segment
 *  |                     |
 *  | Loaded segments     |
 *  | .text, .data, .bss  |
 *  | , etc.              |
 *  |                     |
 *  +---------------------+ <------+ The bottom of Vmar at 0x1_0000
 *  |                     |          64 KiB unusable space
 *  +---------------------+
 *  (low address)
 */

// The process user space virtual memory
pub struct ProcessVm {
    root_vmar: Vmar<Full>,
    heap: Heap,
    stack: Stack,
}

impl Clone for ProcessVm {
    fn clone(&self) -> Self {
        Self {
            root_vmar: self.root_vmar.dup().unwrap(),
            heap: self.heap.clone(),
            stack: self.stack.clone(),
        }
    }
}

impl ProcessVm {
    pub fn alloc() -> Self {
        let root_vmar = Vmar::<Full>::new_root();
        let heap = Heap::new();
        heap.init(&root_vmar);
        let stack = Stack::new(root_vmar.dup().unwrap());
        Self {
            root_vmar,
            heap,
            stack,
        }
    }

    pub fn new(root_vmar: Vmar<Full>, heap: Heap, stack: Stack) -> Self {
        Self {
            root_vmar,
            heap,
            stack,
        }
    }

    pub fn root_vmar(&self) -> &Vmar<Full> {
        &self.root_vmar
    }

    pub(super) fn stack(&self) -> &Stack {
        &self.stack
    }

    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    /// Set the `ProcessVm`` to the init status
    pub fn clear(&self) {
        self.root_vmar.clear().unwrap();
        self.stack.clear();
        self.heap.set_default(&self.root_vmar);
    }
}
