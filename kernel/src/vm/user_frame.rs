use core::ops::Range;

use ostd::{
    mm::{vm_space::VmItem, UFrame},
    task::Task,
};

use super::perms::VmPerms;
use crate::{
    prelude::*, thread::exception::PageFaultInfo, vm::page_fault_handler::PageFaultHandler,
};

/// A `UFrame` behind a user address.
struct UserFrame {
    va: Vaddr,
    frame: UFrame,
    offset: usize,
    len: usize,
}

impl UserFrame {
    /// Retrieves the user frames within the virtual address range.
    pub fn from_range(
        &self,
        mut range: Range<Vaddr>,
        required_perms: VmPerms,
    ) -> Result<Vec<UserFrame>> {
        // If this method is only used in syscalls, we can add a parameter `&Context` and pass
        // the `ctx` directly, then we can use `ctx.user_space()` to get the `user_space`.
        let task = Task::current().unwrap();
        let user_space = CurrentUserSpace::new(&task);

        let mut user_frames = Vec::new();
        let vmar = user_space.root_vmar();
        loop {
            let cursor = vmar.vm_space().cursor(&range)?;
            for vm_item in cursor {
                match vm_item {
                    VmItem::NotMapped { va, len } => {
                        // Drop the cursor to release the spinlock since we may do IO during handling
                        // page fault.
                        drop(cursor);
                        let page_fault_info = PageFaultInfo {
                            address: va,
                            required_perms,
                        };
                        vmar.handle_page_fault(&page_fault_info)?;

                        range.start = va;
                        continue;
                    }
                    VmItem::Mapped { va, frame, prop } => {
                        let mapped_perms = VmPerms::from(prop.flags);
                        if !mapped_perms.contains(required_perms) {
                            // Drop the cursor to release the spinlock since we may do IO during handling
                            // page fault.
                            drop(cursor);
                            let page_fault_info = PageFaultInfo {
                                address: va,
                                required_perms,
                            };
                            vmar.handle_page_fault(&page_fault_info)?;

                            range.start = va;
                            continue;
                        }

                        user_frames.push(UserFrame {
                            va,
                            frame,
                            offset: todo!("calculate"),
                            len: todo!("calculate"),
                        });
                    }
                }
            }
            break;
        }
        Ok(user_frames)
    }
}
