// SPDX-License-Identifier: MPL-2.0

//! Inode cache for `virtiofs`.

use aster_fuse::{EntryReply, FuseNodeId};
use aster_virtio::device::filesystem::device::AttrVersion;
use hashbrown::HashMap;

use super::{VirtioFs, VirtioFsInode};
use crate::prelude::*;

/// In-memory inode cache keyed by FUSE node ID.
pub(in crate::fs::fs_impls::virtiofs) struct InodeCache {
    inodes: RwMutex<HashMap<FuseNodeId, Weak<VirtioFsInode>>>,
}

impl InodeCache {
    pub(in crate::fs::fs_impls::virtiofs) fn new(root: &Arc<VirtioFsInode>) -> Self {
        let inodes = RwMutex::new(HashMap::from_iter([(root.nodeid(), Arc::downgrade(root))]));
        Self { inodes }
    }

    /// Looks up an inode by FUSE node ID and validates its generation.
    pub(in crate::fs::fs_impls::virtiofs) fn lookup_inode(
        &self,
        lookup_reply: EntryReply,
        request_attr_version: AttrVersion,
        fs: &Arc<VirtioFs>,
    ) -> Result<Arc<VirtioFsInode>> {
        let nodeid = lookup_reply.nodeid();

        // An `EntryReply` carries a new lookup reference even when the inode is
        // already cached, so cache hits still need to commit the reply.
        if let Some(inode) = self
            .inodes
            .read()
            .get(&nodeid)
            .and_then(Weak::upgrade)
            .filter(|inode| inode.generation() == lookup_reply.generation())
        {
            // Reusing a cached inode can still fail while committing the
            // returned attributes, because a size or mtime change may require
            // page-cache resize or invalidation. There is no local recovery
            // path for those page-cache errors, so propagate them to the
            // lookup caller.
            inode.update_from_entry_reply(&lookup_reply, request_attr_version)?;
            return Ok(inode);
        }

        // Recheck after taking the write lock because another thread may have
        // inserted the inode while this lookup waited for exclusive access.
        let mut inode_cache = self.inodes.write();
        if let Some(inode) = inode_cache
            .get(&nodeid)
            .and_then(Weak::upgrade)
            .filter(|inode| inode.generation() == lookup_reply.generation())
        {
            drop(inode_cache);
            // See the first cache-hit path above for why committing a fresh
            // `EntryReply` to a cached inode may still fail.
            inode.update_from_entry_reply(&lookup_reply, request_attr_version)?;
            return Ok(inode);
        }

        let inode = VirtioFsInode::new_from_entry_reply(lookup_reply, fs);
        inode_cache.insert(nodeid, Arc::downgrade(&inode));

        Ok(inode)
    }

    /// Inserts an inode by FUSE node ID.
    pub(in crate::fs::fs_impls::virtiofs) fn insert_inode(&self, inode: &Arc<VirtioFsInode>) {
        self.inodes
            .write()
            .insert(inode.nodeid(), Arc::downgrade(inode));
    }

    pub(in crate::fs::fs_impls::virtiofs) fn remove_inode(
        &self,
        inode: &VirtioFsInode,
    ) -> Option<Weak<VirtioFsInode>> {
        let nodeid = inode.nodeid();
        let mut inodes = self.inodes.write();

        if inodes
            .get(&nodeid)
            .is_some_and(|cached_inode| cached_inode.ptr_eq(&inode.weak_self))
        {
            return inodes.remove(&nodeid);
        }

        None
    }
}
