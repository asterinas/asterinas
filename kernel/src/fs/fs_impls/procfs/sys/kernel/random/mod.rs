// SPDX-License-Identifier: MPL-2.0

use aster_util::slot_vec::SlotVec;
use ostd::sync::RwMutexUpgradeableGuard;

use crate::{
    fs::{
        file::mkmod,
        procfs::{
            ProcDir,
            sys::kernel::random::boot_id::BootIdFileOps,
            template::{
                DirOps, ProcDirBuilder, lookup_child_from_table, populate_children_from_table,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
};

mod boot_id;

/// Represents the inode at `/proc/sys/kernel/random`.
pub struct RandomDirOps;

impl RandomDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(Self, mkmod!(a+rx))
            .parent(parent)
            .build()
            .unwrap()
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [(&'static str, fn(Weak<dyn Inode>) -> Arc<dyn Inode>)] = &[
        ("boot_id", BootIdFileOps::new_inode),
    ];
}

impl DirOps for RandomDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let mut cached_children = dir.cached_children().write();

        if let Some(child) =
            lookup_child_from_table(name, &mut cached_children, Self::STATIC_ENTRIES, |f| {
                (f)(dir.this_weak().clone())
            })
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let mut cached_children = dir.cached_children().write();

        populate_children_from_table(&mut cached_children, Self::STATIC_ENTRIES, |f| {
            (f)(dir.this_weak().clone())
        });

        cached_children.downgrade()
    }
}
