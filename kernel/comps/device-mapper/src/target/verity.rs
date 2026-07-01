// SPDX-License-Identifier: MPL-2.0

//! The `verity` target.
//!
//! A read-only target that verifies the integrity of a data device against a
//! precomputed Merkle hash tree stored on a separate hash device. Each data
//! block read is hashed and checked, level by level, up to a trusted root
//! digest supplied out of band (on the kernel command line). Any mismatch
//! fails the read with an I/O error, so tampering with either the data or the
//! hash device is detected before corrupted data is ever returned.
//!
//! The root digest is the root of trust and must be delivered through a trusted
//! path. The target guarantees block integrity against that root, not the
//! authenticity of the root itself, which matches Linux dm-verity.
//!
//! The on-disk format follows Linux dm-verity:
//!
//! - The per-block digest is `sha256(salt || block)` for version 1 and
//!   `sha256(block || salt)` for version 0.
//! - The hash tree is laid out top level first, each level packing as many
//!   fixed-size digests per hash block as fit, with the single top-level block
//!   hashing to the root digest.
//!
//! Reference: Linux `Documentation/admin-guide/device-mapper/verity.rst`
//! (<https://docs.kernel.org/admin-guide/device-mapper/verity.html>).

use alloc::{sync::Arc, vec::Vec};

use aster_block::{
    BLOCK_SIZE, BlockDevice, SECTOR_SIZE,
    bio::{BioStatus, BioType, SubmittedBio},
};
use ostd::mm::VmIo;

use super::DmTarget;
use crate::{
    DmError, DmErrorWithContext,
    parser::{lookup_block_device, parse_field, parse_hex_bytes},
    sha256,
};

/// The digest size of SHA-256, in bytes.
const DIGEST_SIZE: usize = 32;
/// The number of 512-byte sectors per block.
const SECTORS_PER_BLOCK: u64 = (BLOCK_SIZE / SECTOR_SIZE) as u64;
/// The number of mandatory arguments in a `verity` table line.
const NR_TABLE_ARGS: usize = 10;

/// One level of the hash tree, in the order it is stored on the hash device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HashLevel {
    /// The first hash block of the level, relative to the start of the hash
    /// device.
    pub first_block: u64,
    /// The number of hash blocks in the level.
    pub nr_blocks: u64,
}

/// Computes the hash-tree levels for a data device of `num_data_blocks` blocks.
///
/// Levels are returned top level first (the single root block), matching the
/// on-device layout: the data block digests fill the last (leaf) level, every
/// hash block holds up to `hashes_per_block` digests of the level below, and
/// the levels stack contiguously starting at `hash_start_block`.
pub fn build_hash_levels(
    num_data_blocks: u64,
    hashes_per_block: u64,
    hash_start_block: u64,
) -> Result<Vec<HashLevel>, DmError> {
    if num_data_blocks == 0 || hashes_per_block == 0 {
        return Err(DmError::InvalidArgument);
    }

    // Block counts from the leaf level up to (but not including) the root.
    let mut counts_leaf_to_root = Vec::new();
    let mut count = num_data_blocks.div_ceil(hashes_per_block);
    while count > 1 {
        counts_leaf_to_root.push(count);
        count = count.div_ceil(hashes_per_block);
    }
    counts_leaf_to_root.push(1);

    let mut levels = Vec::with_capacity(counts_leaf_to_root.len());
    let mut first_block = hash_start_block;
    for &nr_blocks in counts_leaf_to_root.iter().rev() {
        levels.push(HashLevel {
            first_block,
            nr_blocks,
        });
        first_block = first_block
            .checked_add(nr_blocks)
            .ok_or(DmError::InvalidArgument)?;
    }
    Ok(levels)
}

/// A read-only `verity` target.
#[derive(Debug)]
pub struct VerityTarget {
    data_device: Arc<dyn BlockDevice>,
    hash_device: Arc<dyn BlockDevice>,
    num_data_blocks: u64,
    size_sectors: u64,
    version: u8,
    salt: Vec<u8>,
    root_digest: [u8; DIGEST_SIZE],
    /// Hash-tree levels, top level first.
    levels: Vec<HashLevel>,
}

impl VerityTarget {
    /// Parses the arguments of a `verity` table line.
    ///
    /// The accepted form mirrors the Linux dm-verity table:
    ///
    /// ```text
    /// <version> <data_dev> <hash_dev> <data_block_size> <hash_block_size>
    /// <num_data_blocks> <hash_start_block> <algorithm> <root_digest> <salt>
    /// ```
    ///
    /// Supports SHA-256 with 4096-byte data and hash blocks; the salt may be
    /// `-` to denote an empty salt.
    pub fn from_table_args(args: &[&str]) -> Result<Self, DmErrorWithContext> {
        if args.len() != NR_TABLE_ARGS && args.len() != NR_TABLE_ARGS + 1 {
            return Err(DmError::InvalidTable.context(
                "verity target expects: <version> <data_dev> <hash_dev> \
                 <data_block_size> <hash_block_size> <num_data_blocks> \
                 <hash_start_block> <algorithm> <root_digest> <salt> [0]",
            ));
        }
        if args.len() == NR_TABLE_ARGS + 1 && args[NR_TABLE_ARGS] != "0" {
            return Err(DmError::UnsupportedTarget
                .context("verity target optional parameters are not supported"));
        }

        let version = parse_field::<u8>(args[0], "verity version")?;
        if version > 1 {
            return Err(DmError::InvalidArgument.context("verity version must be 0 or 1"));
        }

        let data_device = lookup_block_device(args[1])?;
        let hash_device = lookup_block_device(args[2])?;

        let data_block_size = parse_field::<usize>(args[3], "verity data block size")?;
        let hash_block_size = parse_field::<usize>(args[4], "verity hash block size")?;
        if data_block_size != BLOCK_SIZE || hash_block_size != BLOCK_SIZE {
            return Err(DmError::InvalidArgument
                .context("verity target only supports 4096-byte data and hash blocks"));
        }

        let num_data_blocks = parse_field::<u64>(args[5], "verity data block count")?;
        if num_data_blocks == 0 {
            return Err(DmError::InvalidArgument.context("verity data block count must be nonzero"));
        }
        let hash_start_block = parse_field::<u64>(args[6], "verity hash start block")?;

        if args[7] != "sha256" {
            return Err(DmError::UnsupportedTarget
                .context("verity target only supports the sha256 algorithm"));
        }

        let root_bytes = parse_hex_bytes(args[8])
            .map_err(|err| err.context("verity root digest is not valid hex"))?;
        if root_bytes.len() != DIGEST_SIZE {
            return Err(
                DmError::InvalidArgument.context("verity root digest must be 32 bytes for sha256")
            );
        }
        let mut root_digest = [0u8; DIGEST_SIZE];
        root_digest.copy_from_slice(&root_bytes);

        let salt =
            parse_hex_bytes(args[9]).map_err(|err| err.context("verity salt is not valid hex"))?;

        let hashes_per_block = (BLOCK_SIZE / DIGEST_SIZE) as u64;
        let levels = build_hash_levels(num_data_blocks, hashes_per_block, hash_start_block)
            .map_err(|err| err.context("verity hash tree geometry is invalid"))?;
        let size_sectors = num_data_blocks
            .checked_mul(SECTORS_PER_BLOCK)
            .ok_or_else(|| DmError::InvalidArgument.context("verity data device is too large"))?;
        if size_sectors > data_device.metadata().nr_sectors as u64 {
            return Err(DmError::InvalidArgument
                .context("verity data device is smaller than the table geometry"));
        }
        let hash_end_block = levels
            .last()
            .and_then(|level| level.first_block.checked_add(level.nr_blocks))
            .ok_or_else(|| DmError::InvalidArgument.context("verity hash tree is too large"))?;
        let hash_end_sector = hash_end_block
            .checked_mul(SECTORS_PER_BLOCK)
            .ok_or_else(|| DmError::InvalidArgument.context("verity hash device is too large"))?;
        if hash_end_sector > hash_device.metadata().nr_sectors as u64 {
            return Err(DmError::InvalidArgument
                .context("verity hash device is smaller than the table geometry"));
        }

        Ok(Self {
            data_device,
            hash_device,
            num_data_blocks,
            size_sectors,
            version,
            salt,
            root_digest,
            levels,
        })
    }

    /// Hashes a block with the salt, ordered per the dm-verity version.
    fn hash_block(&self, block: &[u8]) -> [u8; DIGEST_SIZE] {
        match self.version {
            0 => sha256::digest(&[block, &self.salt]),
            _ => sha256::digest(&[&self.salt, block]),
        }
    }

    /// Verifies `block` (at index `data_block_index`) against the hash tree,
    /// returning whether every level up to the root digest matches.
    ///
    /// `hash_scratch` is a reusable block-sized buffer for reading hash blocks.
    fn verify_block(&self, data_block_index: u64, block: &[u8], hash_scratch: &mut [u8]) -> bool {
        let hashes_per_block = (BLOCK_SIZE / DIGEST_SIZE) as u64;
        let mut child_digest = self.hash_block(block);
        let mut child_index = data_block_index;

        // Walk from the leaf level up to the root.
        for level in self.levels.iter().rev() {
            let block_in_level = child_index / hashes_per_block;
            let slot = (child_index % hashes_per_block) as usize;
            if block_in_level >= level.nr_blocks {
                return false;
            }

            let Some(hash_block_index) = level.first_block.checked_add(block_in_level) else {
                return false;
            };
            let Some(hash_block_offset) = block_offset(hash_block_index) else {
                return false;
            };
            if self
                .hash_device
                .read_bytes(hash_block_offset, hash_scratch)
                .is_err()
            {
                return false;
            }

            let mut stored = [0u8; DIGEST_SIZE];
            stored.copy_from_slice(
                &hash_scratch[slot * DIGEST_SIZE..slot * DIGEST_SIZE + DIGEST_SIZE],
            );
            if stored != child_digest {
                return false;
            }

            child_digest = self.hash_block(hash_scratch);
            child_index = block_in_level;
        }

        // The hash of the top-level block must equal the trusted root digest.
        child_digest == self.root_digest
    }

    fn handle_read(&self, bio: SubmittedBio, target_start_sector: u64) {
        // The verity unit of trust is a full data block, so verification always
        // operates on whole blocks even when the request is sector-granular
        // (the generic partition scanner, for instance, reads a single sector).
        // Each overlapping block is read and verified once, then the requested
        // byte range is scattered into the request's memory segments.
        let Some(mut device_offset) = sector_offset(target_start_sector) else {
            bio.complete(BioStatus::IoError);
            return;
        };

        let mut data_block = super::zero_vec(BLOCK_SIZE);
        let mut hash_scratch = super::zero_vec(BLOCK_SIZE);
        let mut cached_block: Option<u64> = None;

        for segment in bio.segments() {
            let nbytes = segment.nbytes();
            let mut segment_offset = 0;
            while segment_offset < nbytes {
                let block_index = (device_offset / BLOCK_SIZE) as u64;
                if block_index >= self.num_data_blocks {
                    bio.complete(BioStatus::IoError);
                    return;
                }

                if cached_block != Some(block_index) {
                    let Some(block_offset) = block_offset(block_index) else {
                        bio.complete(BioStatus::IoError);
                        return;
                    };
                    if self
                        .data_device
                        .read_bytes(block_offset, &mut data_block)
                        .is_err()
                        || !self.verify_block(block_index, &data_block, &mut hash_scratch)
                    {
                        bio.complete(BioStatus::IoError);
                        return;
                    }
                    cached_block = Some(block_index);
                }

                let within_block = device_offset % BLOCK_SIZE;
                let chunk = (BLOCK_SIZE - within_block).min(nbytes - segment_offset);
                if segment
                    .inner_dma_slice()
                    .write_bytes(
                        segment_offset,
                        &data_block[within_block..within_block + chunk],
                    )
                    .is_err()
                {
                    bio.complete(BioStatus::IoError);
                    return;
                }

                segment_offset += chunk;
                let Some(next_device_offset) = device_offset.checked_add(chunk) else {
                    bio.complete(BioStatus::IoError);
                    return;
                };
                device_offset = next_device_offset;
            }
        }

        bio.complete(BioStatus::Complete);
    }
}

impl DmTarget for VerityTarget {
    fn type_name(&self) -> &'static str {
        "verity"
    }

    fn size_sectors(&self) -> Option<u64> {
        Some(self.size_sectors)
    }

    fn handle_bio(&self, bio: SubmittedBio, target_start_sector: u64) {
        match bio.type_() {
            BioType::Read => self.handle_read(bio, target_start_sector),
            // A verity device is read-only; reject any modification.
            BioType::Write => bio.complete(BioStatus::NotSupported),
            BioType::Flush => bio.complete(BioStatus::Complete),
        }
    }
}

fn sector_offset(sector: u64) -> Option<usize> {
    usize::try_from(sector).ok()?.checked_mul(SECTOR_SIZE)
}

fn block_offset(block: u64) -> Option<usize> {
    usize::try_from(block).ok()?.checked_mul(BLOCK_SIZE)
}
