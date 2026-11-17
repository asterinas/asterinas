// SPDX-License-Identifier: MPL-2.0

//! Internal read-only filesystem state, prepared before publication.
//!
//! The device must retain stable capacity and contents while this instance is
//! used. Opening validates the superblock and a directory root before publishing
//! an instance. The root is strongly owned here and weakly points back to this
//! filesystem. VFS traits are implemented on these same runtime objects.

use aster_block::BlockDevice;
use hashbrown::HashMap;

use super::{
    compressed::PclusterManager,
    inode::{
        ErofsInode,
        desc::{InodeDesc, InodeDescError},
        reader::Nid,
    },
    meta::MetadataCache,
    ondisk::super_block::{SuperBlock, SuperBlockError},
};
use crate::{
    fs::{
        file::InodeType,
        vfs::file_system::{FsEventSubscriberStats, FsFlags},
    },
    prelude::{Arc, RwMutex, Weak},
};

/// A read-only filesystem owning its device and validated configuration.
#[derive(Debug)]
pub(super) struct Erofs {
    /// Backing device shared with the metadata cache backend.
    block_device: Arc<dyn BlockDevice>,
    metadata_cache: Arc<MetadataCache>,
    pcluster: PclusterManager,
    super_block: SuperBlock,
    vfs_super_block: crate::fs::vfs::file_system::SuperBlock,
    flags: FsFlags,
    fs_event_subscriber_stats: FsEventSubscriberStats,
    self_ref: Weak<Self>,
    root: Arc<ErofsInode>,
    inodes: RwMutex<InodeCache>,
}

impl Erofs {
    /// Opens a read-only filesystem with a validated superblock and directory root.
    ///
    /// Cache construction, superblock, and root errors preserve their causes,
    /// including device errors, corrupt metadata, and unsupported formats.
    /// A non-directory root returns
    /// [`ErofsOpenError::RootNotDirectory`].
    ///
    /// The device's capacity and contents must remain stable during opening and
    /// for as long as the returned filesystem is used.
    pub(super) fn open(device: Arc<dyn BlockDevice>) -> Result<Arc<Self>, ErofsOpenError> {
        let super_block = SuperBlock::load(device.as_ref()).map_err(ErofsOpenError::SuperBlock)?;
        let metadata_cache =
            Arc::new(MetadataCache::new(device.clone()).map_err(ErofsOpenError::Cache)?);
        let root_nid = Nid::new(u64::from(super_block.root_nid()));
        let description = InodeDesc::load_cached(&metadata_cache, &super_block, root_nid)
            .map_err(ErofsOpenError::Root)?;
        if description.inode_type() != InodeType::Dir {
            return Err(ErofsOpenError::RootNotDirectory);
        }

        let root_metadata = ErofsInode::prepare_metadata(&description, &super_block, device.id())
            .map_err(ErofsOpenError::Projection)?;
        let vfs_super_block = Self::prepare_vfs_super_block(&super_block, device.id())
            .map_err(ErofsOpenError::Projection)?;
        let pcluster = PclusterManager::new(device.clone()).map_err(ErofsOpenError::Cache)?;

        // Metadata reads, validation, and fallible cache construction precede
        // cyclic allocation so no partially validated filesystem is published.
        let fs = Arc::new_cyclic(|self_ref| {
            let root = ErofsInode::new(
                description,
                root_metadata,
                self_ref.clone(),
                metadata_cache.backend().clone(),
            );
            let inodes = HashMap::from_iter([(root_nid, Arc::downgrade(&root))]);
            Self {
                block_device: device,
                metadata_cache,
                pcluster,
                super_block,
                vfs_super_block,
                flags: FsFlags::RDONLY,
                fs_event_subscriber_stats: FsEventSubscriberStats::new(),
                self_ref: self_ref.clone(),
                root,
                inodes: RwMutex::new(InodeCache::new(inodes)),
            }
        });
        Ok(fs)
    }

    /// Returns the validated directory root retained by this filesystem.
    pub(super) fn root_inode(&self) -> Arc<ErofsInode> {
        self.root.clone()
    }

    /// Returns this filesystem's canonical live inode for a node identifier.
    ///
    /// Callers requesting the same NID share a live object. An inode whose
    /// strong references were all released may be rebuilt. Loading errors retain
    /// device, corrupt-metadata, and unsupported-format causes and are not cached.
    ///
    /// Dead weak entries are reclaimed opportunistically during successful
    /// publications. An inactive cache may retain them, and memory use has no
    /// hard bound while arbitrarily many inodes remain live.
    pub(super) fn load_inode(&self, nid: Nid) -> Result<Arc<ErofsInode>, InodeDescError> {
        let cached = self.inodes.read().entries.get(&nid).and_then(Weak::upgrade);
        if let Some(inode) = cached {
            return Ok(inode);
        }

        // Read/parse and candidate construction stay outside the cache guard.
        // Concurrent misses can duplicate work, so publication rechecks identity.
        let description = InodeDesc::load_cached(&self.metadata_cache, self.super_block(), nid)?;
        let metadata = ErofsInode::prepare_metadata(
            &description,
            self.super_block(),
            self.block_device().id(),
        )
        .map_err(InodeDescError::Projection)?;
        let candidate = ErofsInode::new(
            description,
            metadata,
            self.weak_self(),
            self.metadata_cache.backend().clone(),
        );

        let mut inodes = self.inodes.write();
        if let Some(inode) = inodes.entries.get(&nid).and_then(Weak::upgrade) {
            // The lock is dropped before the losing candidate is released.
            return Ok(inode);
        }
        // Only the winning publication advances reclamation, including stale
        // replacements of the same key. Hits, failures and losers do not.
        let cache = &mut *inodes;
        cache.cleanup.record_publication(&mut cache.entries);
        cache.entries.insert(nid, Arc::downgrade(&candidate));
        Ok(candidate)
    }

    /// Returns the backing device without creating another owner.
    pub(super) fn block_device(&self) -> &dyn BlockDevice {
        self.block_device.as_ref()
    }

    /// Returns the instance-local pcluster manager.
    pub(super) fn pcluster(&self) -> &PclusterManager {
        &self.pcluster
    }

    /// Returns the shared metadata page cache.
    pub(super) fn metadata_cache(&self) -> &Arc<MetadataCache> {
        &self.metadata_cache
    }

    /// Returns the immutable validated configuration.
    pub(super) fn super_block(&self) -> &SuperBlock {
        &self.super_block
    }

    /// Returns immutable VFS statistics validated before publication.
    pub(super) fn vfs_super_block(&self) -> &crate::fs::vfs::file_system::SuperBlock {
        &self.vfs_super_block
    }

    /// Returns the inherent read-only filesystem flags.
    pub(super) fn fs_flags(&self) -> FsFlags {
        self.flags
    }

    /// Returns the existing VFS event subscriber accounting object.
    pub(super) fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }

    /// Returns a back-pointer that does not keep the filesystem alive.
    pub(super) fn weak_self(&self) -> Weak<Self> {
        self.self_ref.clone()
    }
}

// An activity threshold, not a memory limit or a measured optimum.
const MIN_CLEANUP_INTERVAL: usize = 64;

#[derive(Debug)]
struct InodeCache {
    entries: HashMap<Nid, Weak<ErofsInode>>,
    cleanup: WeakCacheCleanup,
}

impl InodeCache {
    fn new(entries: HashMap<Nid, Weak<ErofsInode>>) -> Self {
        // The prevalidated root is seeded, not counted as a runtime publication.
        Self {
            entries,
            cleanup: WeakCacheCleanup::new(),
        }
    }
}

/// Per-map activity state for reclaiming expired inode weak references.
#[derive(Debug)]
pub(super) struct WeakCacheCleanup {
    publications_since_cleanup: usize,
    cleanup_interval: usize,
}

impl WeakCacheCleanup {
    pub(super) fn new() -> Self {
        Self {
            publications_since_cleanup: 0,
            cleanup_interval: MIN_CLEANUP_INTERVAL,
        }
    }

    /// Reclaims dead entries when a successful publication reaches its interval.
    ///
    /// The caller must retain this policy for the same map and hold its write lock.
    pub(super) fn record_publication<T>(&mut self, entries: &mut HashMap<Nid, Weak<T>>) {
        self.publications_since_cleanup = self.publications_since_cleanup.saturating_add(1);
        if self.publications_since_cleanup < self.cleanup_interval {
            return;
        }

        // Entries refer only to fully published objects. Inspect counts instead
        // of upgrading: dropping a temporary last Arc could run its destructor
        // while the cache write guard is held. Concurrent deaths can wait for
        // the next sweep; no live object is evicted.
        entries.retain(|_, inode| inode.strong_count() != 0);
        self.publications_since_cleanup = 0;
        // These counts exclude the candidate about to be inserted. Larger live
        // sets amortize scanning; shrink only with substantial unused capacity.
        let live_entries = entries.len();
        self.cleanup_interval = MIN_CLEANUP_INTERVAL.max(live_entries.saturating_mul(2));
        let shrink_threshold = MIN_CLEANUP_INTERVAL.max(live_entries.saturating_mul(4));
        if entries.capacity() > shrink_threshold {
            entries.shrink_to(MIN_CLEANUP_INTERVAL.max(live_entries.saturating_mul(2)));
        }
    }
}

/// An opening failure preserving cache construction and metadata validation causes.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum ErofsOpenError {
    SuperBlock(SuperBlockError),
    Cache(ostd::Error),
    Root(InodeDescError),
    RootNotDirectory,
    Projection(ostd::Error),
}
