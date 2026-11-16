// SPDX-License-Identifier: MPL-2.0

//! Raw EROFS superblock layout, field validation, and checked snapshot loading.
//!
//! Format reference: <https://erofs.docs.kernel.org/en/latest/ondisk/core_ondisk.html>.
//! Raw fields are untrusted; `SuperBlock::load` checks the superblock and any
//! declared LZ4 configuration from the same stable device snapshot.
//! Referenced metadata objects still require validation by their consumers.

use core::mem::offset_of;

use aster_block::BlockDevice;
use ostd::{const_assert, mm::VmIo};

use super::{super::io, checksum, compression::RawLz4Config};
use crate::prelude::Pod;

pub(in super::super) const SUPER_BLOCK_OFFSET_BYTES: usize = 1024;
pub(in super::super) const SUPER_BLOCK_BASE_SIZE_BYTES: usize = 128;
pub(in super::super) const SUPER_BLOCK_EXTENSION_SLOT_SIZE_BYTES: usize = 16;
pub(in super::super) const EROFS_SUPER_MAGIC_V1: u32 = 0xE0F5_E1E2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in super::super) struct RawSuperBlock {
    magic: u32,
    checksum: u32,
    feature_compat: u32,
    blkszbits: u8,
    sb_extslots: u8,
    rootnid_2b: u16,
    inos: u64,
    epoch: u64,
    fixed_nsec: u32,
    blocks: u32,
    meta_blkaddr: u32,
    xattr_blkaddr: u32,
    uuid: [u8; 16],
    volume_name: [u8; 16],
    feature_incompat: u32,
    compression_union: u16,
    dontcare_56: [u8; 4],
    dirblkbits: u8,
    dontcare_5b: [u8; 5],
    dontcare_60: [u8; 8],
    xattr_filter_reserved: u8,
    dontcare_69: u8,
    reserved_6a: [u8; 2],
    dontcare_6c: [u8; 12],
    reserved_78: [u8; 8],
}

impl RawSuperBlock {
    pub(in super::super) const fn magic(&self) -> u32 {
        u32::from_le(self.magic)
    }

    pub(in super::super) const fn checksum(&self) -> u32 {
        u32::from_le(self.checksum)
    }

    pub(in super::super) const fn feature_compat(&self) -> u32 {
        u32::from_le(self.feature_compat)
    }

    pub(in super::super) const fn root_nid(&self) -> u16 {
        u16::from_le(self.rootnid_2b)
    }

    pub(in super::super) const fn inode_count(&self) -> u64 {
        u64::from_le(self.inos)
    }

    pub(in super::super) const fn epoch(&self) -> u64 {
        u64::from_le(self.epoch)
    }

    pub(in super::super) const fn fixed_nsec(&self) -> u32 {
        u32::from_le(self.fixed_nsec)
    }

    pub(in super::super) const fn blocks(&self) -> u32 {
        u32::from_le(self.blocks)
    }

    pub(in super::super) const fn meta_blkaddr(&self) -> u32 {
        u32::from_le(self.meta_blkaddr)
    }

    pub(in super::super) const fn xattr_blkaddr(&self) -> u32 {
        u32::from_le(self.xattr_blkaddr)
    }

    pub(in super::super) const fn feature_incompat(&self) -> u32 {
        u32::from_le(self.feature_incompat)
    }

    /// Returns the legacy LZ4 distance or the COMPR_CFGS algorithm bitmask.
    pub(in super::super) const fn compression_union(&self) -> u16 {
        u16::from_le(self.compression_union)
    }

    pub(in super::super) const fn block_size_bits(&self) -> u8 {
        self.blkszbits
    }

    pub(in super::super) const fn extension_slot_count(&self) -> u8 {
        self.sb_extslots
    }

    pub(in super::super) const fn directory_block_bits(&self) -> u8 {
        self.dirblkbits
    }

    #[expect(dead_code)]
    pub(in super::super) const fn xattr_filter_reserved(&self) -> u8 {
        self.xattr_filter_reserved
    }

    pub(in super::super) const fn reserved_6a(&self) -> [u8; 2] {
        self.reserved_6a
    }

    pub(in super::super) const fn reserved_78(&self) -> [u8; 8] {
        self.reserved_78
    }
}

const_assert!(size_of::<RawSuperBlock>() == SUPER_BLOCK_BASE_SIZE_BYTES);
const_assert!(offset_of!(RawSuperBlock, magic) == 0x00);
const_assert!(offset_of!(RawSuperBlock, checksum) == 0x04);
const_assert!(offset_of!(RawSuperBlock, feature_compat) == 0x08);
const_assert!(offset_of!(RawSuperBlock, blkszbits) == 0x0c);
const_assert!(offset_of!(RawSuperBlock, sb_extslots) == 0x0d);
const_assert!(offset_of!(RawSuperBlock, rootnid_2b) == 0x0e);
const_assert!(offset_of!(RawSuperBlock, inos) == 0x10);
const_assert!(offset_of!(RawSuperBlock, epoch) == 0x18);
const_assert!(offset_of!(RawSuperBlock, fixed_nsec) == 0x20);
const_assert!(offset_of!(RawSuperBlock, blocks) == 0x24);
const_assert!(offset_of!(RawSuperBlock, meta_blkaddr) == 0x28);
const_assert!(offset_of!(RawSuperBlock, xattr_blkaddr) == 0x2c);
const_assert!(offset_of!(RawSuperBlock, uuid) == 0x30);
const_assert!(offset_of!(RawSuperBlock, volume_name) == 0x40);
const_assert!(offset_of!(RawSuperBlock, feature_incompat) == 0x50);
const_assert!(offset_of!(RawSuperBlock, compression_union) == 0x54);
const_assert!(offset_of!(RawSuperBlock, dirblkbits) == 0x5a);
const_assert!(offset_of!(RawSuperBlock, xattr_filter_reserved) == 0x68);
const_assert!(offset_of!(RawSuperBlock, reserved_6a) == 0x6a);
const_assert!(offset_of!(RawSuperBlock, reserved_78) == 0x78);
const_assert!(offset_of!(RawSuperBlock, dontcare_56) == 0x56);
const_assert!(offset_of!(RawSuperBlock, dontcare_5b) == 0x5b);
const_assert!(offset_of!(RawSuperBlock, dontcare_60) == 0x60);
const_assert!(offset_of!(RawSuperBlock, dontcare_69) == 0x69);
const_assert!(offset_of!(RawSuperBlock, dontcare_6c) == 0x6c);

const FEATURE_INCOMPAT_ZERO_PADDING: u32 = 0x0001;
const FEATURE_INCOMPAT_COMPR_CFGS: u32 = 0x0002;
const FEATURE_INCOMPAT_ZTAILPACKING: u32 = 0x0010;
const SUPPORTED_INCOMPAT_FEATURES: u32 =
    FEATURE_INCOMPAT_ZERO_PADDING | FEATURE_INCOMPAT_COMPR_CFGS | FEATURE_INCOMPAT_ZTAILPACKING;

const MIN_BLOCK_SIZE_BITS: u8 = 9;
const SUPPORTED_BLOCK_SIZE_BITS: u8 = 12;
pub(in super::super) const SUPPORTED_BLOCK_SIZE_BYTES: usize = 4096;
const FEATURE_COMPAT_METABOX: u32 = 0x0000_0008;
const FEATURE_COMPAT_SB_CHKSUM: u32 = 0x0000_0001;
const FEATURE_COMPAT_MTIME: u32 = 0x0000_0002; //bit2=1：mtime，bit2=0：ctime

/// An immutable configuration validated from the same stable device snapshot.
///
/// Success covers superblock fields, declared extension availability, the
/// enabled checksum, and the supported LZ4 configuration when declared.
/// It does not validate referenced inodes or file data.
#[derive(Debug)]
pub(in super::super) struct SuperBlock {
    fields: SuperBlockFields,
    has_lz4_config: bool,
}

impl SuperBlock {
    /// Loads and validates a superblock from a stable device snapshot.
    pub(in super::super) fn load(device: &dyn BlockDevice) -> Result<Self, SuperBlockError> {
        let coverage = io::read_superblock_coverage(device).map_err(SuperBlockError::Device)?;
        let raw = RawSuperBlock::from_bytes(&coverage[..SUPER_BLOCK_BASE_SIZE_BYTES]);
        let fields = SuperBlockFields::parse(&raw)?;
        checksum::verify_superblock_checksum(&coverage, &fields)
            .map_err(|_| SuperBlockError::Corrupted(SuperBlockCorruption::ChecksumMismatch))?;
        let has_lz4_config = Self::load_compression_config(device, &fields)?;
        Ok(Self {
            fields,
            has_lz4_config,
        })
    }

    /// Validates the declared algorithm configuration without reading file input.
    fn load_compression_config(
        device: &dyn BlockDevice,
        fields: &SuperBlockFields,
    ) -> Result<bool, SuperBlockError> {
        let features = fields.feature_incompat;
        if features == 0 && fields.compression_union == 0 {
            return Ok(false);
        }
        if features & FEATURE_INCOMPAT_ZERO_PADDING == 0 {
            return Err(SuperBlockError::Unsupported(UnsupportedField::Compression));
        }
        if features & FEATURE_INCOMPAT_COMPR_CFGS == 0 {
            // The union is a legacy distance hint. Zero means the LZ4 default;
            // every u16 distance is supported by the full-block decoder.
            return Ok(true);
        }
        if fields.compression_union != 1 {
            return Err(SuperBlockError::Unsupported(
                UnsupportedField::CompressionAlgorithm,
            ));
        }
        // erofs_read_metadata aligns the length prefix to four bytes. The
        // superblock base and extension slots already provide that alignment.
        let start_bytes = SUPER_BLOCK_OFFSET_BYTES + fields.declared_superblock_size_bytes;
        let nr_sectors = device.metadata().nr_sectors;
        io::check_read_range(nr_sectors, start_bytes, 2).map_err(SuperBlockError::Device)?;
        let mut length = [0; 2];
        device
            .read_bytes(start_bytes, &mut length)
            .map_err(SuperBlockError::Device)?;
        let encoded_length_bytes = u16::from_le_bytes(length);
        let length_bytes = if encoded_length_bytes == 0 {
            65536
        } else {
            usize::from(encoded_length_bytes)
        };
        if length_bytes < size_of::<RawLz4Config>() {
            return Err(SuperBlockError::Corrupted(
                SuperBlockCorruption::CompressionConfig,
            ));
        }
        let body_start_bytes = start_bytes + 2;
        io::check_read_range(nr_sectors, body_start_bytes, length_bytes)
            .map_err(SuperBlockError::Device)?;
        let mut bytes = [0; size_of::<RawLz4Config>()];
        device
            .read_bytes(body_start_bytes, &mut bytes)
            .map_err(SuperBlockError::Device)?;
        let mut extension = [0; 256];
        let mut consumed_bytes = bytes.len();
        while consumed_bytes < length_bytes {
            let window_bytes = (length_bytes - consumed_bytes).min(extension.len());
            device
                .read_bytes(
                    body_start_bytes + consumed_bytes,
                    &mut extension[..window_bytes],
                )
                .map_err(SuperBlockError::Device)?;
            consumed_bytes += window_bytes;
        }
        let config = RawLz4Config::from_bytes(&bytes);
        // Zero is the specified one-block default. Future record extensions
        // and reserved bytes do not change the understood LZ4 prefix.
        if config.max_pcluster_blocks() > 1 {
            return Err(SuperBlockError::Unsupported(UnsupportedField::BigPcluster));
        }
        Ok(true)
    }

    pub(in super::super) const fn has_lz4_config(&self) -> bool {
        self.has_lz4_config
    }

    pub(in super::super) const fn has_compressed_inline(&self) -> bool {
        self.fields.feature_incompat & FEATURE_INCOMPAT_ZTAILPACKING != 0
    }

    /// Returns the filesystem block size in bytes.
    pub(in super::super) const fn block_size_bytes(&self) -> usize {
        self.fields.block_size_bytes()
    }

    /// Returns the directory block size in bytes.
    #[expect(dead_code)]
    pub(in super::super) const fn directory_block_size_bytes(&self) -> usize {
        self.fields.directory_block_size_bytes()
    }

    /// Returns the root identifier, without validating its target.
    pub(in super::super) const fn root_nid(&self) -> u16 {
        self.fields.root_nid()
    }

    /// Returns the metadata starting block, without validating its target.
    pub(in super::super) const fn metadata_start_block(&self) -> u32 {
        self.fields.metadata_start_block()
    }

    /// Returns the shared metadata block identifier, without checking its target.
    pub(in super::super) const fn xattr_start_block(&self) -> u32 {
        self.fields.xattr_start_block
    }

    /// Returns whether shared entries require the unsupported metabox space.
    pub(in super::super) const fn has_metabox_xattrs(&self) -> bool {
        self.fields.feature_compat & FEATURE_COMPAT_METABOX != 0
    }

    /// Returns the reported block count, which is not an address bound.
    pub(in super::super) const fn reported_blocks(&self) -> u32 {
        self.fields.reported_blocks()
    }

    /// Returns the reported inode count, which is not a NID bound.
    pub(in super::super) const fn reported_inodes(&self) -> u64 {
        self.fields.reported_inodes()
    }

    /// Returns the raw seconds value for later timestamp conversion.
    pub(in super::super) const fn epoch_seconds(&self) -> u64 {
        self.fields.epoch_seconds()
    }

    /// Returns the raw nanosecond component without normalizing it.
    pub(in super::super) const fn fixed_nsec(&self) -> u32 {
        self.fields.fixed_nsec()
    }

    /// Returns whether the image declares a checksum that was verified.
    #[expect(dead_code)]
    pub(in super::super) const fn has_superblock_checksum(&self) -> bool {
        self.fields.has_superblock_checksum()
    }

    /// Returns whether inode timestamps declare modification-time semantics.
    pub(in super::super) const fn uses_mtime(&self) -> bool {
        self.fields.uses_mtime()
    }
}

/// A superblock loading failure preserving the failing stage's cause.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum SuperBlockError {
    Device(ostd::Error),
    Corrupted(SuperBlockCorruption),
    Unsupported(UnsupportedField),
}

/// A violated field constraint or a failed checksum.
#[derive(Debug, Eq, PartialEq)]
pub(in super::super) enum SuperBlockCorruption {
    Field(CorruptedField),
    ChecksumMismatch,
    CompressionConfig,
}

impl From<SuperBlockFieldError> for SuperBlockError {
    fn from(error: SuperBlockFieldError) -> Self {
        match error {
            SuperBlockFieldError::Corrupted(field) => {
                Self::Corrupted(SuperBlockCorruption::Field(field))
            }
            SuperBlockFieldError::Unsupported(field) => Self::Unsupported(field),
        }
    }
}

/// A configuration that has passed only the base-field checks.
///
/// Addresses are still unverified identifiers. Statistics are report values,
/// not address bounds. Time fields retain their on-disk values without range
/// validation or conversion into system timestamps.
#[derive(Debug)]
pub(in super::super) struct SuperBlockFields {
    declared_superblock_size_bytes: usize,
    root_nid: u16,
    metadata_start_block: u32,
    xattr_start_block: u32,
    feature_compat: u32,
    feature_incompat: u32,
    compression_union: u16,
    recorded_checksum: u32,
    reported_blocks: u32,
    reported_inodes: u64,
    epoch_seconds: u64,
    fixed_nsec: u32,
}

impl TryFrom<&RawSuperBlock> for SuperBlockFields {
    type Error = SuperBlockFieldError;

    fn try_from(raw: &RawSuperBlock) -> Result<Self, Self::Error> {
        Self::parse(raw)
    }
}

impl SuperBlockFields {
    fn parse(raw: &RawSuperBlock) -> Result<Self, SuperBlockFieldError> {
        if raw.magic() != EROFS_SUPER_MAGIC_V1 {
            return Err(SuperBlockFieldError::Corrupted(CorruptedField::BadMagic));
        }

        let block_size_bits = raw.block_size_bits();
        if block_size_bits < MIN_BLOCK_SIZE_BITS {
            return Err(SuperBlockFieldError::Corrupted(
                CorruptedField::InvalidBlockSizeBits,
            ));
        }

        // Only 4096-byte filesystem blocks are supported.
        if block_size_bits != SUPPORTED_BLOCK_SIZE_BITS {
            return Err(SuperBlockFieldError::Unsupported(
                UnsupportedField::BlockSize,
            ));
        }

        // Compatible hints may be ignored. Consumers reject unsupported
        // semantics such as shared metabox xattrs when they are encountered.
        // Incompatible extensions must be understood before reinterpreting any
        // of the fields that follow.
        if raw.feature_incompat() & !SUPPORTED_INCOMPAT_FEATURES != 0 {
            return Err(SuperBlockFieldError::Unsupported(
                UnsupportedField::IncompatibleFeatures,
            ));
        }
        if raw.directory_block_bits() != 0 {
            return Err(SuperBlockFieldError::Unsupported(
                UnsupportedField::DirectoryBlockSize,
            ));
        }

        // The xattr format defines 0x68 as filter_reserved: a nonzero value
        // disables Bloom filtering. Complete entry scans ignore that hint,
        // including when the compatible filter bit is absent.
        // Other reserved bytes retain their must-zero requirements.
        if raw.reserved_6a() != [0; 2] || raw.reserved_78() != [0; 8] {
            return Err(SuperBlockFieldError::Corrupted(
                CorruptedField::NonzeroReserved,
            ));
        }

        let invalid_slots = SuperBlockFieldError::Corrupted(CorruptedField::InvalidExtensionSlots);
        let declared_superblock_size_bytes = usize::from(raw.extension_slot_count())
            .checked_mul(SUPER_BLOCK_EXTENSION_SLOT_SIZE_BYTES)
            .and_then(|extension_bytes| SUPER_BLOCK_BASE_SIZE_BYTES.checked_add(extension_bytes))
            .ok_or(invalid_slots)?;
        let declared_end_bytes = SUPER_BLOCK_OFFSET_BYTES
            .checked_add(declared_superblock_size_bytes)
            .ok_or(invalid_slots)?;
        if declared_end_bytes > SUPPORTED_BLOCK_SIZE_BYTES {
            return Err(invalid_slots);
        }

        Ok(Self {
            declared_superblock_size_bytes,
            root_nid: raw.root_nid(),
            metadata_start_block: raw.meta_blkaddr(),
            xattr_start_block: raw.xattr_blkaddr(),
            feature_compat: raw.feature_compat(),
            feature_incompat: raw.feature_incompat(),
            compression_union: raw.compression_union(),
            recorded_checksum: raw.checksum(),
            reported_blocks: raw.blocks(),
            reported_inodes: raw.inode_count(),
            epoch_seconds: raw.epoch(),
            fixed_nsec: raw.fixed_nsec(),
        })
    }
}

impl SuperBlockFields {
    /// Returns the supported filesystem block size in bytes.
    pub(in super::super) const fn block_size_bytes(&self) -> usize {
        SUPPORTED_BLOCK_SIZE_BYTES
    }

    /// Returns the supported directory block size in bytes.
    #[expect(dead_code)]
    pub(in super::super) const fn directory_block_size_bytes(&self) -> usize {
        SUPPORTED_BLOCK_SIZE_BYTES
    }

    /// Returns the declared length, without proving extension bytes exist.
    #[expect(dead_code)]
    pub(in super::super) const fn declared_superblock_size_bytes(&self) -> usize {
        self.declared_superblock_size_bytes
    }

    /// Returns the root identifier, without validating its target.
    pub(in super::super) const fn root_nid(&self) -> u16 {
        self.root_nid
    }

    /// Returns the unverified metadata starting block number.
    pub(in super::super) const fn metadata_start_block(&self) -> u32 {
        self.metadata_start_block
    }

    /// Returns all compatible feature bits, including unknown bits.
    #[expect(dead_code)]
    pub(in super::super) const fn feature_compat(&self) -> u32 {
        self.feature_compat
    }

    /// Returns whether the image declares a checksum requiring later validation.
    pub(in super::super) const fn has_superblock_checksum(&self) -> bool {
        self.feature_compat & FEATURE_COMPAT_SB_CHKSUM != 0
    }

    /// Returns the recorded checksum value, without verifying it.
    pub(in super::super) const fn recorded_checksum(&self) -> u32 {
        self.recorded_checksum
    }

    /// Returns whether inode timestamps declare modification-time semantics.
    pub(in super::super) const fn uses_mtime(&self) -> bool {
        self.feature_compat & FEATURE_COMPAT_MTIME != 0
    }

    /// Returns the reported block count, which is not an address bound.
    pub(in super::super) const fn reported_blocks(&self) -> u32 {
        self.reported_blocks
    }

    /// Returns the reported inode count, which is not a NID bound.
    pub(in super::super) const fn reported_inodes(&self) -> u64 {
        self.reported_inodes
    }

    /// Returns the on-disk seconds value without validating its time range.
    pub(in super::super) const fn epoch_seconds(&self) -> u64 {
        self.epoch_seconds
    }

    /// Returns the on-disk nanosecond component without normalizing it.
    pub(in super::super) const fn fixed_nsec(&self) -> u32 {
        self.fixed_nsec
    }
}

/// A base-field failure, independent of device and checksum validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum SuperBlockFieldError {
    Corrupted(CorruptedField),
    Unsupported(UnsupportedField),
}

/// A violated base-format constraint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum CorruptedField {
    BadMagic,
    InvalidBlockSizeBits,
    NonzeroReserved,
    InvalidExtensionSlots,
}

/// A field requesting a configuration outside the supported subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum UnsupportedField {
    BlockSize,
    IncompatibleFeatures,
    DirectoryBlockSize,
    Compression,
    CompressionAlgorithm,
    BigPcluster,
}
