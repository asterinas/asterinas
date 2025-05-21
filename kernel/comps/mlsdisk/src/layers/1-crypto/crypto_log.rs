// SPDX-License-Identifier: MPL-2.0

use alloc::vec;
use core::{any::Any, mem::size_of};

use ostd::const_assert;
use ostd_pod::Pod;
use serde::{Deserialize, Serialize};

use super::{Iv, Key, Mac};
use crate::{
    layers::bio::{BlockId, BlockLog, Buf, BufMut, BufRef, BLOCK_SIZE},
    os::{Aead, HashMap, Mutex, RwLock},
    prelude::*,
};

/// A cryptographically-protected log of user data blocks.
///
/// `CryptoLog<L>`, which is backed by an untrusted block log (`L`),
/// serves as a secure log file that supports random reads and append-only
/// writes of data blocks. `CryptoLog<L>` encrypts the data blocks and
/// protects them with a Merkle Hash Tree (MHT), which itself is also encrypted.
///
/// # Security
///
/// Each instance of `CryptoLog<L>` is assigned a randomly-generated root key
/// upon its creation. The root key is used to encrypt the root MHT block only.
/// Each new version of the root MHT block is encrypted with the same key, but
/// different random IVs. This arrangement ensures the confidentiality of
/// the root block.
///
/// After flushing a `CryptoLog<L>`, a new root MHT (as well as other MHT nodes)
/// shall be appended to the backend block log (`L`).
/// The metadata of the root MHT, including its position, encryption
/// key, IV, and MAC, must be kept by the user of `CryptoLog<L>` so that
/// he or she can use the metadata to re-open the `CryptoLog`.
/// The information contained in the metadata is sufficient to verify the
/// integrity and freshness of the root MHT node, and thus the whole `CryptoLog`.
///
/// Other MHT nodes as well as data nodes are encrypted with randomly-generated,
/// unique keys. Their metadata, including its position, encryption key, IV, and
/// MAC, are kept securely in their parent MHT nodes, which are also encrypted.
/// Thus, the confidentiality and integrity of non-root nodes are protected.
///
/// # Performance
///
/// Thanks to its append-only nature, `CryptoLog<L>` avoids MHT's high
/// performance overheads under the workload of random writes
/// due to "cascades of updates".
///
/// Behind the scene, `CryptoLog<L>` keeps a cache for nodes so that frequently
/// or lately accessed nodes can be found in the cache, avoiding the I/O
/// and decryption cost incurred when re-reading these nodes.
/// The cache is also used for buffering new data so that multiple writes to
/// individual nodes can be merged into a large write to the underlying block log.
/// Therefore, `CryptoLog<L>` is efficient for both reads and writes.
///
/// # Disk space
///
/// One consequence of using an append-only block log (`L`) as the backend is
/// that `CryptoLog<L>` cannot do in-place updates to existing MHT nodes.
/// This means the new version of MHT nodes are appended to the underlying block
/// log and the invalid blocks occupied by old versions are not reclaimed.
///
/// But lucky for us, this block reclamation problem is not an issue in practice.
/// This is because a `CryptoLog<L>` is created for one of the following two
/// use cases.
///
/// 1. Write-once-then-read-many. In this use case, all the content of a
///    `CryptoLog` is written in a single run.
///    Writing in a single run won't trigger any updates to MHT nodes and thus
///    no waste of disk space.
///    After the writing is done, the `CryptoLog` becomes read-only.
///
/// 2. Write-many-then-read-once. In this use case, the content of a
///    `CryptoLog` may be written in many runs. But the number of `CryptoLog`
///    under such workloads is limited and their lengths are also limited.
///    So the disk space wasted by such `CryptoLog` is bounded.
///    And after such `CryptoLog`s are done writing, they will be read once and
///    then discarded.
pub struct CryptoLog<L> {
    mht: RwLock<Mht<L>>,
}

type Lbid = BlockId; // Logical block position, in terms of user
type Pbid = BlockId; // Physical block position, in terms of underlying log
type Height = u8; // The height of the MHT

/// A Merkle-Hash Tree (MHT).
struct Mht<L> {
    root: Option<(RootMhtMeta, Arc<MhtNode>)>,
    root_key: Key,
    data_buf: AppendDataBuf<L>,
    storage: Arc<MhtStorage<L>>,
}

/// Storage medium for MHT, including both in-memory and persistent.
struct MhtStorage<L> {
    block_log: L,
    node_cache: Arc<dyn NodeCache>,
    crypt_buf: Mutex<CryptBuf>,
}

/// The metadata of the root MHT node of a `CryptoLog`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootMhtMeta {
    pub pos: Pbid,
    pub mac: Mac,
    pub iv: Iv,
}

/// The Merkle-Hash Tree (MHT) node (internal).
/// It contains a header for node metadata and a bunch of entries for managing children nodes.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct MhtNode {
    header: MhtNodeHeader,
    entries: [MhtNodeEntry; MHT_NBRANCHES],
}
const_assert!(size_of::<MhtNode>() <= BLOCK_SIZE);

/// The header contains metadata of the current MHT node.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct MhtNodeHeader {
    // The height of the MHT whose root is this node
    height: Height,
    // The total number of valid data nodes covered by this node
    num_data_nodes: u32,
    // The number of valid entries (children) within this node
    num_valid_entries: u16,
}

/// The entry of the MHT node, which contains the
/// metadata of the child MHT/data node.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct MhtNodeEntry {
    pos: Pbid,
    key: Key,
    mac: Mac,
}

// Number of branches of one MHT node. (102 for now)
const MHT_NBRANCHES: usize = (BLOCK_SIZE - size_of::<MhtNodeHeader>()) / size_of::<MhtNodeEntry>();

/// The data node (leaf). It contains a block of data.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct DataNode([u8; BLOCK_SIZE]);

/// Builder for MHT.
struct TreeBuilder<'a, L> {
    previous_build: Option<PreviousBuild<'a, L>>,
    storage: &'a MhtStorage<L>,
}

/// Builder for one specific level of MHT.
struct LevelBuilder {
    level_entries: Vec<MhtNodeEntry>,
    total_data_nodes: usize,
    height: Height,
    previous_incomplete_node: Option<Arc<MhtNode>>,
}

/// It encloses necessary information of the previous build of MHT.
struct PreviousBuild<'a, L> {
    root: Arc<MhtNode>,
    height: Height,
    // Each level at most have one incomplete node at end
    internal_incomplete_nodes: HashMap<Height, Arc<MhtNode>>,
    storage: &'a MhtStorage<L>,
}

/// The node cache used by `CryptoLog`. User-defined node cache
/// can achieve TX-awareness.
pub trait NodeCache: Send + Sync {
    /// Gets an owned value from cache corresponding to the position.
    fn get(&self, pos: Pbid) -> Option<Arc<dyn Any + Send + Sync>>;

    /// Puts a position-value pair into cache. If the value of that position
    /// already exists, updates it and returns the old value. Otherwise, `None` is returned.
    fn put(
        &self,
        pos: Pbid,
        value: Arc<dyn Any + Send + Sync>,
    ) -> Option<Arc<dyn Any + Send + Sync>>;
}

/// Context for a search request.
struct SearchCtx<'a> {
    pub pos: Lbid,
    pub data_buf: BufMut<'a>,
    pub offset: usize,
    pub num: usize,
    pub is_completed: bool,
}

/// Prepares buffer for node cryption.
struct CryptBuf {
    pub plain: Buf,
    pub cipher: Buf,
}

impl<L: BlockLog> CryptoLog<L> {
    /// Creates a new `CryptoLog`.
    ///
    /// A newly-created instance won't occupy any space on the `block_log`
    /// until the first flush, which triggers writing the root MHT node.
    pub fn new(block_log: L, root_key: Key, node_cache: Arc<dyn NodeCache>) -> Self {
        Self {
            mht: RwLock::new(Mht::new(block_log, root_key, node_cache)),
        }
    }

    /// Opens an existing `CryptoLog` backed by a `block_log`.
    ///
    /// The given key and the metadata of the root MHT are sufficient to
    /// load and verify the root node of the `CryptoLog`.
    pub fn open(
        block_log: L,
        root_key: Key,
        root_meta: RootMhtMeta,
        node_cache: Arc<dyn NodeCache>,
    ) -> Result<Self> {
        Ok(Self {
            mht: RwLock::new(Mht::open(block_log, root_key, root_meta, node_cache)?),
        })
    }

    /// Gets the root key.
    pub fn root_key(&self) -> Key {
        self.mht.read().root_key
    }

    /// Gets the metadata of the root MHT node.
    ///
    /// Returns `None` if there hasn't been any appends or flush.
    pub fn root_meta(&self) -> Option<RootMhtMeta> {
        self.mht.read().root_meta()
    }

    fn root_node(&self) -> Option<Arc<MhtNode>> {
        self.mht.read().root_node().cloned()
    }

    /// Gets the number of data nodes (blocks).
    pub fn nblocks(&self) -> usize {
        self.mht.read().total_data_nodes()
    }

    /// Reads one or multiple data blocks at a specified position.
    pub fn read(&self, pos: Lbid, buf: BufMut) -> Result<()> {
        let mut search_ctx = SearchCtx::new(pos, buf);
        self.mht.read().search(&mut search_ctx)?;

        debug_assert!(search_ctx.is_completed);
        Ok(())
    }

    /// Appends one or multiple data blocks at the end.
    pub fn append(&self, buf: BufRef) -> Result<()> {
        let data_nodes: Vec<Arc<DataNode>> = buf
            .iter()
            .map(|block_buf| {
                let data_node = {
                    let mut node = DataNode::new_uninit();
                    node.0.copy_from_slice(block_buf.as_slice());
                    Arc::new(node)
                };
                data_node
            })
            .collect();

        self.mht.write().append_data_nodes(data_nodes)
    }

    /// Ensures that all new data are persisted.
    ///
    /// Each successful flush triggers writing a new version of the root MHT
    /// node to the underlying block log. The metadata of the latest root MHT
    /// can be obtained via the `root_meta` method.
    pub fn flush(&self) -> Result<()> {
        self.mht.write().flush()
    }

    pub fn display_mht(&self) {
        self.mht.read().display();
    }
}

impl<L: BlockLog> Mht<L> {
    // Buffer capacity for appended data nodes.
    const APPEND_BUF_CAPACITY: usize = 2048;

    pub fn new(block_log: L, root_key: Key, node_cache: Arc<dyn NodeCache>) -> Self {
        let storage = Arc::new(MhtStorage::new(block_log, node_cache));
        let start_pos = 0 as Lbid;
        Self {
            root: None,
            root_key,
            data_buf: AppendDataBuf::new(Self::APPEND_BUF_CAPACITY, start_pos, storage.clone()),
            storage,
        }
    }

    pub fn open(
        block_log: L,
        root_key: Key,
        root_meta: RootMhtMeta,
        node_cache: Arc<dyn NodeCache>,
    ) -> Result<Self> {
        let storage = Arc::new(MhtStorage::new(block_log, node_cache));
        let root_node = storage.root_mht_node(&root_key, &root_meta)?;
        let start_pos = root_node.num_data_nodes() as Lbid;
        Ok(Self {
            root: Some((root_meta, root_node)),
            root_key,
            data_buf: AppendDataBuf::new(Self::APPEND_BUF_CAPACITY, start_pos, storage.clone()),
            storage,
        })
    }

    pub fn root_meta(&self) -> Option<RootMhtMeta> {
        self.root.as_ref().map(|(root_meta, _)| root_meta.clone())
    }

    fn root_node(&self) -> Option<&Arc<MhtNode>> {
        self.root.as_ref().map(|(_, root_node)| root_node)
    }

    pub fn total_data_nodes(&self) -> usize {
        self.data_buf.num_append()
            + self
                .root
                .as_ref()
                .map_or(0, |(_, root_node)| root_node.num_data_nodes())
    }

    pub fn search(&self, search_ctx: &mut SearchCtx<'_>) -> Result<()> {
        let root_node = self
            .root_node()
            .ok_or(Error::with_msg(NotFound, "root MHT node not found"))?;

        if search_ctx.pos + search_ctx.num > self.total_data_nodes() {
            return_errno_with_msg!(InvalidArgs, "search out of MHT capacity");
        }

        // Search the append data buffer first
        self.data_buf.search_data_nodes(search_ctx)?;
        if search_ctx.is_completed {
            return Ok(());
        }

        // Search the MHT if needed
        self.search_hierarchy(vec![root_node.clone()], root_node.height(), search_ctx)
    }

    fn search_hierarchy(
        &self,
        level_targets: Vec<Arc<MhtNode>>,
        mut curr_height: Height,
        search_ctx: &mut SearchCtx<'_>,
    ) -> Result<()> {
        debug_assert!(
            !level_targets.is_empty() && curr_height == level_targets.first().unwrap().height()
        );
        let num_data_nodes = search_ctx.num;

        // Calculate two essential values for searching the current level:
        // how many nodes to skip and how many nodes we need
        let (nodes_skipped, nodes_needed) = {
            let pos = &mut search_ctx.pos;
            let next_level_max_num_data_nodes = MhtNode::max_num_data_nodes(curr_height - 1);
            let skipped = *pos / next_level_max_num_data_nodes;
            *pos -= skipped * next_level_max_num_data_nodes;
            let needed = align_up(num_data_nodes + *pos, next_level_max_num_data_nodes)
                / next_level_max_num_data_nodes;
            (skipped, needed)
        };

        let target_entries = level_targets
            .iter()
            .flat_map(|node| node.entries.iter())
            .skip(nodes_skipped)
            .take(nodes_needed);

        // Search down to the leaves, ready to collect data nodes
        if MhtNode::is_lowest_level(curr_height) {
            debug_assert_eq!(num_data_nodes, nodes_needed);
            for entry in target_entries {
                self.storage
                    .read_data_node(entry, search_ctx.node_buf(search_ctx.offset))?;
                search_ctx.offset += 1;
            }
            search_ctx.is_completed = true;
            return Ok(());
        }

        // Prepare target MHT nodes for the lower level
        let next_level_targets = {
            let mut targets = Vec::with_capacity(nodes_needed);
            for entry in target_entries {
                let target_node = self.storage.read_mht_node(
                    entry.pos,
                    &entry.key,
                    &entry.mac,
                    &Iv::new_zeroed(),
                )?;
                targets.push(target_node);
            }
            targets
        };

        // Search the lower level
        curr_height -= 1;
        self.search_hierarchy(next_level_targets, curr_height, search_ctx)
    }

    pub fn append_data_nodes(&mut self, data_nodes: Vec<Arc<DataNode>>) -> Result<()> {
        self.data_buf.append_data_nodes(data_nodes)?;
        if self.data_buf.is_full() {
            let data_node_entries = self.data_buf.flush()?;
            self.do_build(data_node_entries)?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        let data_node_entries = self.data_buf.flush()?;
        self.do_build(data_node_entries)?;
        // FIXME: Should we sync the storage here?
        // self.storage.flush()?;
        Ok(())
    }

    fn do_build(&mut self, data_node_entries: Vec<MhtNodeEntry>) -> Result<()> {
        let new_root_node = {
            TreeBuilder::new(&self.storage)
                .previous_built_root(self.root.as_ref().map(|(_, root_node)| root_node))
                .build(data_node_entries)?
        };
        let root_meta = self
            .storage
            .append_root_mht_node(&self.root_key, &new_root_node)?;
        let _ = self.root.insert((root_meta, new_root_node));
        Ok(())
    }

    pub fn display(&self) {
        info!("{:?}", MhtDisplayer(self));
    }
}

impl<L: BlockLog> MhtStorage<L> {
    pub fn new(block_log: L, node_cache: Arc<dyn NodeCache>) -> Self {
        Self {
            block_log,
            node_cache,
            crypt_buf: Mutex::new(CryptBuf::new()),
        }
    }

    pub fn flush(&self) -> Result<()> {
        self.block_log.flush()
    }

    pub fn root_mht_node(&self, root_key: &Key, root_meta: &RootMhtMeta) -> Result<Arc<MhtNode>> {
        self.read_mht_node(root_meta.pos, root_key, &root_meta.mac, &root_meta.iv)
    }

    pub fn append_root_mht_node(&self, root_key: &Key, node: &Arc<MhtNode>) -> Result<RootMhtMeta> {
        let mut crypt_buf = self.crypt_buf.lock();
        let iv = Iv::random();
        let mac = Aead::new().encrypt(
            node.as_bytes(),
            root_key,
            &iv,
            &[],
            crypt_buf.cipher.as_mut_slice(),
        )?;

        let pos = self.block_log.append(crypt_buf.cipher.as_ref())?;
        self.node_cache.put(pos, node.clone());
        Ok(RootMhtMeta { pos, mac, iv })
    }

    fn append_mht_nodes(&self, nodes: &[Arc<MhtNode>]) -> Result<Vec<MhtNodeEntry>> {
        let num_append = nodes.len();
        let mut node_entries = Vec::with_capacity(num_append);
        let mut cipher_buf = Buf::alloc(num_append)?;
        let mut pos = self.block_log.nblocks() as BlockId;
        let start_pos = pos;
        for (i, node) in nodes.iter().enumerate() {
            let plain = node.as_bytes();
            let cipher = &mut cipher_buf.as_mut_slice()[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE];
            let key = Key::random();
            let mac = Aead::new().encrypt(plain, &key, &Iv::new_zeroed(), &[], cipher)?;

            node_entries.push(MhtNodeEntry { pos, key, mac });
            self.node_cache.put(pos, node.clone());
            pos += 1;
        }

        let append_pos = self.block_log.append(cipher_buf.as_ref())?;
        debug_assert_eq!(start_pos, append_pos);
        Ok(node_entries)
    }

    fn append_data_nodes(&self, nodes: &[Arc<DataNode>]) -> Result<Vec<MhtNodeEntry>> {
        let num_append = nodes.len();
        let mut node_entries = Vec::with_capacity(num_append);
        if num_append == 0 {
            return Ok(node_entries);
        }

        let mut cipher_buf = Buf::alloc(num_append)?;
        let mut pos = self.block_log.nblocks() as BlockId;
        let start_pos = pos;
        for (i, node) in nodes.iter().enumerate() {
            let cipher = &mut cipher_buf.as_mut_slice()[i * BLOCK_SIZE..(i + 1) * BLOCK_SIZE];
            let key = Key::random();
            let mac = Aead::new().encrypt(&node.0, &key, &Iv::new_zeroed(), &[], cipher)?;

            node_entries.push(MhtNodeEntry { pos, key, mac });
            pos += 1;
        }

        let append_pos = self.block_log.append(cipher_buf.as_ref())?;
        debug_assert_eq!(start_pos, append_pos);
        Ok(node_entries)
    }

    fn read_mht_node(&self, pos: Pbid, key: &Key, mac: &Mac, iv: &Iv) -> Result<Arc<MhtNode>> {
        if let Some(node) = self.node_cache.get(pos) {
            return node.downcast::<MhtNode>().map_err(|_| {
                Error::with_msg(InvalidArgs, "cache node downcasts to MHT node failed")
            });
        }

        let mht_node = {
            let mut crypt_buf = self.crypt_buf.lock();
            self.block_log.read(pos, crypt_buf.cipher.as_mut())?;
            let mut node = MhtNode::new_zeroed();
            Aead::new().decrypt(
                crypt_buf.cipher.as_slice(),
                key,
                iv,
                &[],
                mac,
                node.as_bytes_mut(),
            )?;
            crypt_buf
                .plain
                .as_mut_slice()
                .copy_from_slice(node.as_bytes());
            Arc::new(node)
        };

        self.node_cache.put(pos, mht_node.clone());
        Ok(mht_node)
    }

    fn read_data_node(&self, entry: &MhtNodeEntry, node_buf: &mut [u8]) -> Result<()> {
        debug_assert_eq!(node_buf.len(), BLOCK_SIZE);
        let mut crypt_buf = self.crypt_buf.lock();

        self.block_log.read(entry.pos, crypt_buf.cipher.as_mut())?;
        Aead::new().decrypt(
            crypt_buf.cipher.as_slice(),
            &entry.key,
            &Iv::new_zeroed(),
            &[],
            &entry.mac,
            node_buf,
        )
    }
}

impl MhtNode {
    pub fn height(&self) -> Height {
        self.header.height
    }

    pub fn num_data_nodes(&self) -> usize {
        self.header.num_data_nodes as _
    }

    pub fn num_valid_entries(&self) -> usize {
        self.header.num_valid_entries as _
    }

    // Lowest level MHT node's children are data nodes
    pub fn is_lowest_level(height: Height) -> bool {
        height == 1
    }

    pub fn max_num_data_nodes(height: Height) -> usize {
        // Also correct when height equals 0
        MHT_NBRANCHES.pow(height as _)
    }

    // A complete node indicates that all children are valid and
    // all covered with maximum number of data nodes
    pub fn is_incomplete(&self) -> bool {
        self.num_data_nodes() != Self::max_num_data_nodes(self.height())
    }

    pub fn num_complete_children(&self) -> usize {
        if self.num_data_nodes() % MHT_NBRANCHES == 0 || Self::is_lowest_level(self.height()) {
            self.num_valid_entries()
        } else {
            self.num_valid_entries() - 1
        }
    }
}

impl<'a, L: BlockLog> TreeBuilder<'a, L> {
    pub fn new(storage: &'a MhtStorage<L>) -> Self {
        Self {
            previous_build: None,
            storage,
        }
    }

    pub fn previous_built_root(mut self, previous_built_root: Option<&Arc<MhtNode>>) -> Self {
        if previous_built_root.is_none() {
            return self;
        }
        self.previous_build = Some(PreviousBuild::new(
            previous_built_root.unwrap(),
            self.storage,
        ));
        self
    }

    pub fn build(&self, data_node_entries: Vec<MhtNodeEntry>) -> Result<Arc<MhtNode>> {
        let total_data_nodes = data_node_entries.len()
            + self
                .previous_build
                .as_ref()
                .map_or(0, |pre| pre.num_data_nodes());

        self.build_hierarchy(
            data_node_entries,
            total_data_nodes,
            1 as Height,
            self.calc_target_height(total_data_nodes),
        )
    }

    fn build_hierarchy(
        &self,
        level_entries: Vec<MhtNodeEntry>,
        total_data_nodes: usize,
        mut curr_height: Height,
        target_height: Height,
    ) -> Result<Arc<MhtNode>> {
        // Build the MHT nodes of current level
        let mut new_mht_nodes = {
            // Previous built incomplete node of same level should participate in the building
            let previous_incomplete_node = self
                .previous_build
                .as_ref()
                .and_then(|pre| pre.target_node(curr_height));

            LevelBuilder::new(level_entries, total_data_nodes, curr_height)
                .previous_incomplete_node(previous_incomplete_node)
                .build()
        };

        if curr_height == target_height {
            // The root MHT node has been built
            debug_assert_eq!(new_mht_nodes.len(), 1);
            return Ok(new_mht_nodes.pop().unwrap());
        }

        // Prepare MHT node entries for the higher level
        let next_level_entries = self.storage.append_mht_nodes(&new_mht_nodes)?;
        // Build the higher level
        curr_height += 1;
        self.build_hierarchy(
            next_level_entries,
            total_data_nodes,
            curr_height,
            target_height,
        )
    }

    fn calc_target_height(&self, num_data_node_entries: usize) -> Height {
        let target_height = num_data_node_entries.ilog(MHT_NBRANCHES);
        if MHT_NBRANCHES.pow(target_height) < num_data_node_entries || target_height == 0 {
            (target_height + 1) as Height
        } else {
            target_height as Height
        }
    }
}

impl LevelBuilder {
    pub fn new(level_entries: Vec<MhtNodeEntry>, total_data_nodes: usize, height: Height) -> Self {
        Self {
            level_entries,
            total_data_nodes,
            height,
            previous_incomplete_node: None,
        }
    }

    pub fn previous_incomplete_node(
        mut self,
        previous_incomplete_node: Option<Arc<MhtNode>>,
    ) -> Self {
        self.previous_incomplete_node = previous_incomplete_node;
        self
    }

    pub fn build(&self) -> Vec<Arc<MhtNode>> {
        let all_level_entries: Vec<&MhtNodeEntry> =
            if let Some(pre_node) = self.previous_incomplete_node.as_ref() {
                // If there exists a previous built node (same height),
                // its complete entries should participate in the building
                pre_node
                    .entries
                    .iter()
                    .take(pre_node.num_complete_children())
                    .chain(self.level_entries.iter())
                    .collect()
            } else {
                self.level_entries.iter().collect()
            };

        let num_build = align_up(all_level_entries.len(), MHT_NBRANCHES) / MHT_NBRANCHES;
        let mut new_mht_nodes = Vec::with_capacity(num_build);
        // Each iteration builds a new MHT node
        for (nth, entries_per_node) in all_level_entries.chunks(MHT_NBRANCHES).enumerate() {
            if nth == num_build - 1 {
                let last_new_node = self.build_last_node(entries_per_node);
                new_mht_nodes.push(last_new_node);
                break;
            }

            let mut mht_node = MhtNode::new_zeroed();
            mht_node.header = MhtNodeHeader {
                height: self.height,
                num_data_nodes: MhtNode::max_num_data_nodes(self.height) as _,
                num_valid_entries: MHT_NBRANCHES as _,
            };
            for (i, entry) in mht_node.entries.iter_mut().enumerate() {
                *entry = *entries_per_node[i];
            }

            new_mht_nodes.push(Arc::new(mht_node));
        }
        new_mht_nodes
    }

    // Building last MHT node of the level can be complicated, since
    // the last node may be incomplete
    fn build_last_node(&self, entries: &[&MhtNodeEntry]) -> Arc<MhtNode> {
        let num_data_nodes = {
            let max_data_nodes = MhtNode::max_num_data_nodes(self.height);
            let incomplete_nodes = self.total_data_nodes % max_data_nodes;
            if incomplete_nodes == 0 {
                max_data_nodes
            } else {
                incomplete_nodes
            }
        };
        let num_valid_entries = entries.len();

        let mut last_mht_node = MhtNode::new_zeroed();
        last_mht_node.header = MhtNodeHeader {
            height: self.height,
            num_data_nodes: num_data_nodes as _,
            num_valid_entries: num_valid_entries as _,
        };
        for (i, entry) in last_mht_node.entries.iter_mut().enumerate() {
            *entry = if i < num_valid_entries {
                *entries[i]
            } else {
                // Padding invalid entries to the rest
                MhtNodeEntry::new_uninit()
            };
        }

        Arc::new(last_mht_node)
    }
}

impl<'a, L: BlockLog> PreviousBuild<'a, L> {
    pub fn new(previous_built_root: &Arc<MhtNode>, storage: &'a MhtStorage<L>) -> Self {
        let mut new_self = Self {
            root: previous_built_root.clone(),
            height: previous_built_root.height(),
            internal_incomplete_nodes: HashMap::new(),
            storage,
        };
        new_self.collect_incomplete_nodes();
        new_self
    }

    pub fn target_node(&self, target_height: Height) -> Option<Arc<MhtNode>> {
        if target_height == self.height {
            return Some(self.root.clone());
        }
        self.internal_incomplete_nodes.get(&target_height).cloned()
    }

    pub fn num_data_nodes(&self) -> usize {
        self.root.num_data_nodes()
    }

    fn collect_incomplete_nodes(&mut self) {
        let root_node = &self.root;
        if !root_node.is_incomplete() || MhtNode::is_lowest_level(self.height) {
            return;
        }

        let mut lookup_node = {
            let entry = root_node.entries[root_node.num_valid_entries() - 1];
            self.storage
                .read_mht_node(entry.pos, &entry.key, &entry.mac, &Iv::new_zeroed())
                .unwrap()
        };

        while lookup_node.is_incomplete() {
            let height = lookup_node.height();
            self.internal_incomplete_nodes
                .insert(height, lookup_node.clone());

            if MhtNode::is_lowest_level(height) {
                break;
            }

            // Incomplete nodes only appear in the last node of each level
            lookup_node = {
                let entry = lookup_node.entries[lookup_node.num_valid_entries() - 1];
                self.storage
                    .read_mht_node(entry.pos, &entry.key, &entry.mac, &Iv::new_zeroed())
                    .unwrap()
            }
        }
    }
}

impl<'a> SearchCtx<'a> {
    pub fn new(pos: Lbid, data_buf: BufMut<'a>) -> Self {
        let num = data_buf.nblocks();
        Self {
            pos,
            data_buf,
            offset: 0,
            num,
            is_completed: false,
        }
    }

    pub fn node_buf(&mut self, offset: usize) -> &mut [u8] {
        &mut self.data_buf.as_mut_slice()[offset * BLOCK_SIZE..(offset + 1) * BLOCK_SIZE]
    }
}

impl CryptBuf {
    pub fn new() -> Self {
        Self {
            plain: Buf::alloc(1).unwrap(),
            cipher: Buf::alloc(1).unwrap(),
        }
    }
}

/// A buffer that contains appended data.
struct AppendDataBuf<L> {
    node_queue: Vec<Arc<DataNode>>,
    node_queue_cap: usize,
    entry_queue: Vec<MhtNodeEntry>, // Also cache the data node entries
    entry_queue_cap: usize,
    start_pos: Lbid,
    storage: Arc<MhtStorage<L>>,
}

impl<L: BlockLog> AppendDataBuf<L> {
    // Maximum capacity of entries indicates a complete MHT (height equals 3)
    const MAX_ENTRY_QUEUE_CAP: usize = MHT_NBRANCHES.pow(3);

    pub fn new(capacity: usize, start_pos: Lbid, storage: Arc<MhtStorage<L>>) -> Self {
        let (node_queue_cap, entry_queue_cap) = Self::calc_queue_cap(capacity, start_pos);
        Self {
            node_queue: Vec::with_capacity(node_queue_cap),
            node_queue_cap,
            entry_queue: Vec::with_capacity(entry_queue_cap),
            start_pos,
            entry_queue_cap,
            storage,
        }
    }

    pub fn num_append(&self) -> usize {
        self.node_queue.len() + self.entry_queue.len()
    }

    pub fn is_full(&self) -> bool {
        // Returns whether the data node entry queue is at capacity
        self.entry_queue.len() >= self.entry_queue_cap
    }

    pub fn append_data_nodes(&mut self, nodes: Vec<Arc<DataNode>>) -> Result<()> {
        if self.is_full() {
            return_errno_with_msg!(OutOfMemory, "cache out of capacity");
        }

        self.node_queue.extend(nodes);
        if self.node_queue.len() >= self.node_queue_cap {
            // If node queue is full, flush nodes to the entry queue
            self.flush_node_queue()?;
        }
        Ok(())
    }

    pub fn search_data_nodes(&self, search_ctx: &mut SearchCtx) -> Result<()> {
        let start_pos = self.start_pos;
        let (pos, num) = (search_ctx.pos, search_ctx.num);
        if pos + num <= start_pos {
            return Ok(());
        }

        let (mut start_nth, mut end_nth, mut offset) = if pos >= start_pos {
            let start = pos - start_pos;
            (start, start + num, 0)
        } else {
            let end = pos + num - start_pos;
            let offset = search_ctx.num - end;
            search_ctx.num -= end;
            (0, end, offset)
        };
        debug_assert!(end_nth <= self.num_append());

        // Read from entry queue first if needed
        for entry in self
            .entry_queue
            .iter()
            .skip(start_nth)
            .take(end_nth - start_nth)
        {
            self.storage
                .read_data_node(entry, search_ctx.node_buf(offset))?;
            start_nth = 0;
            end_nth -= 1;
            offset += 1;
        }

        // Read from node queue if needed
        for node in self
            .node_queue
            .iter()
            .skip(start_nth)
            .take(end_nth - start_nth)
        {
            let node_buf = search_ctx.node_buf(offset);
            node_buf.copy_from_slice(&node.0);
            offset += 1;
        }

        if pos >= start_pos {
            search_ctx.is_completed = true;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<Vec<MhtNodeEntry>> {
        self.flush_node_queue()?;
        debug_assert!(self.node_queue.is_empty());

        let all_cached_entries: Vec<MhtNodeEntry> = self.entry_queue.drain(..).collect();
        self.start_pos += all_cached_entries.len();
        Ok(all_cached_entries)
    }

    fn flush_node_queue(&mut self) -> Result<()> {
        let new_node_entries = self.storage.append_data_nodes(&self.node_queue)?;
        self.entry_queue.extend_from_slice(&new_node_entries);
        self.node_queue.clear();
        Ok(())
    }

    fn calc_queue_cap(capacity: usize, append_pos: Lbid) -> (usize, usize) {
        // Half for data nodes, half for data node entries
        let node_queue_cap = capacity / 2;
        let entry_queue_cap = {
            let max_cap = Self::MAX_ENTRY_QUEUE_CAP - append_pos;
            let remain_cap = (capacity - node_queue_cap) * BLOCK_SIZE / size_of::<MhtNodeEntry>();
            max_cap.min(remain_cap)
        };
        (node_queue_cap, entry_queue_cap)
    }
}

impl<L: BlockLog> Debug for CryptoLog<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CryptoLog")
            .field("mht", &self.mht.read())
            .finish()
    }
}

impl<L: BlockLog> Debug for Mht<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mht")
            .field("root_meta", &self.root_meta())
            .field("root_node", &self.root_node())
            .field("root_key", &self.root_key)
            .field("total_data_nodes", &self.total_data_nodes())
            .field("buffered_data_nodes", &self.data_buf.num_append())
            .finish()
    }
}

impl Debug for MhtNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MhtNode")
            .field("header", &self.header)
            .finish()
    }
}

impl Debug for DataNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataNode")
            .field("first 16 bytes", &&self.0[..16])
            .finish()
    }
}

struct MhtDisplayer<'a, L>(&'a Mht<L>);

impl<L: BlockLog> Debug for MhtDisplayer<'_, L> {
    // A heavy implementation to display the whole MHT.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug_struct = f.debug_struct("Mht");

        // Display root MHT node
        let root_meta = self.0.root_meta();
        debug_struct.field("\nroot_meta", &root_meta);
        if root_meta.is_none() {
            return debug_struct.finish();
        }
        let root_mht_node = self.0.root_node().unwrap();
        debug_struct.field("\n-> root_mht_node", &root_mht_node);
        let mut height = root_mht_node.height();
        if MhtNode::is_lowest_level(height) {
            return debug_struct.finish();
        }

        // Display internal MHT nodes hierarchically
        let mut level_entries: Vec<MhtNodeEntry> = root_mht_node
            .entries
            .into_iter()
            .take(root_mht_node.num_valid_entries())
            .collect();
        'outer: loop {
            let level_size = level_entries.len();
            for i in 0..level_size {
                let entry = &level_entries[i];
                let node = self
                    .0
                    .storage
                    .read_mht_node(entry.pos, &entry.key, &entry.mac, &Iv::new_zeroed())
                    .unwrap();
                debug_struct.field("\n node_entry", entry);
                debug_struct.field("\n -> mht_node", &node);
                for i in 0..node.num_valid_entries() {
                    level_entries.push(node.entries[i]);
                }
            }
            level_entries.drain(..level_size);
            height -= 1;
            if MhtNode::is_lowest_level(height) {
                break 'outer;
            }
        }
        debug_struct.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::bio::MemLog;

    struct NoCache;
    impl NodeCache for NoCache {
        fn get(&self, _pos: Pbid) -> Option<Arc<dyn Any + Send + Sync>> {
            None
        }
        fn put(
            &self,
            _pos: Pbid,
            _value: Arc<dyn Any + Send + Sync>,
        ) -> Option<Arc<dyn Any + Send + Sync>> {
            None
        }
    }

    fn create_crypto_log() -> Result<CryptoLog<MemLog>> {
        let mem_log = MemLog::create(64 * 1024)?;
        let key = Key::random();
        let cache = Arc::new(NoCache {});
        Ok(CryptoLog::new(mem_log, key, cache))
    }

    #[test]
    fn crypto_log_fns() -> Result<()> {
        let log = create_crypto_log()?;
        let append_cnt = MHT_NBRANCHES - 1;
        let mut buf = Buf::alloc(1)?;
        for i in 0..append_cnt {
            buf.as_mut_slice().fill(i as _);
            log.append(buf.as_ref())?;
        }
        log.flush()?;
        println!("{:?}", log);
        log.display_mht();

        let content = 5u8;
        buf.as_mut_slice().fill(content);
        log.append(buf.as_ref())?;
        log.flush()?;
        log.display_mht();
        log.append(buf.as_ref())?;
        log.flush()?;
        log.display_mht();

        let (root_meta, root_node) = (log.root_meta().unwrap(), log.root_node().unwrap());
        assert_eq!(root_meta.pos, 107);
        assert_eq!(root_node.height(), 2);
        assert_eq!(root_node.num_data_nodes(), append_cnt + 2);
        assert_eq!(root_node.num_valid_entries(), 2);

        log.read(5 as BlockId, buf.as_mut())?;
        assert_eq!(buf.as_slice(), &[content; BLOCK_SIZE]);
        let mut buf = Buf::alloc(2)?;
        log.read((MHT_NBRANCHES - 1) as BlockId, buf.as_mut())?;
        assert_eq!(buf.as_slice(), &[content; 2 * BLOCK_SIZE]);
        Ok(())
    }

    #[test]
    fn write_once_read_many() -> Result<()> {
        let log = create_crypto_log()?;
        let append_cnt = MHT_NBRANCHES * MHT_NBRANCHES;
        let batch_cnt = 4;
        let mut buf = Buf::alloc(batch_cnt)?;

        for i in 0..(append_cnt / batch_cnt) {
            buf.as_mut_slice().fill(i as _);
            log.append(buf.as_ref())?;
        }
        log.flush()?;
        log.display_mht();

        for i in (0..append_cnt).step_by(batch_cnt) {
            log.read(i as Lbid, buf.as_mut())?;
            assert_eq!(&buf.as_slice()[..128], &[(i / batch_cnt) as u8; 128]);
        }
        Ok(())
    }

    #[test]
    fn write_many_read_once() -> Result<()> {
        let log = create_crypto_log()?;
        let append_cnt = 2048;
        let flush_freq = 125;
        let mut buf = Buf::alloc(1)?;

        for i in 0..append_cnt {
            buf.as_mut_slice().fill(i as _);
            log.append(buf.as_ref())?;
            if i % flush_freq == 0 {
                log.flush()?;
            }
        }
        log.flush()?;
        log.display_mht();

        for i in (0..append_cnt).rev() {
            log.read(i as Lbid, buf.as_mut())?;
            assert_eq!(&buf.as_slice()[2048..], &[i as u8; 2048]);
        }
        Ok(())
    }
}
