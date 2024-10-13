// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Display, ops::Range};

use aster_rights::Full;
use ostd::mm::VmIo;

use super::{
    constants::{EXFAT_FILE_NAME_LEN, MAX_NAME_LENGTH},
    fat::FatChainFlags,
    fs::ExfatFS,
    inode::FatAttr,
    upcase_table::ExfatUpcaseTable,
    utils::{calc_checksum_16, DosTimestamp},
};
use crate::{
    fs::utils::{InodeMode, InodeType},
    prelude::*,
    vm::vmo::Vmo,
};

pub(super) const DENTRY_SIZE: usize = 32; // directory entry size

#[derive(Debug, Clone, Copy)]
pub(super) enum ExfatDentry {
    File(ExfatFileDentry),
    Stream(ExfatStreamDentry),
    Name(ExfatNameDentry),
    Bitmap(ExfatBitmapDentry),
    Upcase(ExfatUpcaseDentry),
    VendorExt(ExfatVendorExtDentry),
    VendorAlloc(ExfatVendorAllocDentry),
    GenericPrimary(ExfatGenericPrimaryDentry),
    GenericSecondary(ExfatGenericSecondaryDentry),
    Deleted(ExfatDeletedDentry),
    UnUsed,
}

impl ExfatDentry {
    fn as_le_bytes(&self) -> &[u8] {
        match self {
            ExfatDentry::File(file) => file.as_bytes(),
            ExfatDentry::Stream(stream) => stream.as_bytes(),
            ExfatDentry::Name(name) => name.as_bytes(),
            ExfatDentry::Bitmap(bitmap) => bitmap.as_bytes(),
            ExfatDentry::Upcase(upcase) => upcase.as_bytes(),
            ExfatDentry::VendorExt(vendor_ext) => vendor_ext.as_bytes(),
            ExfatDentry::GenericPrimary(primary) => primary.as_bytes(),
            ExfatDentry::GenericSecondary(secondary) => secondary.as_bytes(),
            ExfatDentry::Deleted(deleted) => deleted.as_bytes(),
            _ => &[0; DENTRY_SIZE],
        }
    }
}

const EXFAT_UNUSED: u8 = 0x00;

#[allow(dead_code)]
const EXFAT_INVAL: u8 = 0x80;
const EXFAT_BITMAP: u8 = 0x81;
#[allow(dead_code)]
const EXFAT_UPCASE: u8 = 0x82;
#[allow(dead_code)]
const EXFAT_VOLUME: u8 = 0x83;
const EXFAT_FILE: u8 = 0x85;

#[allow(dead_code)]
const EXFAT_GUID: u8 = 0xA0;
#[allow(dead_code)]
const EXFAT_PADDING: u8 = 0xA1;
#[allow(dead_code)]
const EXFAT_ACLTAB: u8 = 0xA2;

const EXFAT_STREAM: u8 = 0xC0;
const EXFAT_NAME: u8 = 0xC1;
#[allow(dead_code)]
const EXFAT_ACL: u8 = 0xC2;

const EXFAT_VENDOR_EXT: u8 = 0xE0;
const EXFAT_VENDOR_ALLOC: u8 = 0xE1;

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct RawExfatDentry {
    pub(super) dentry_type: u8,
    pub(super) value: [u8; 31],
}

impl TryFrom<RawExfatDentry> for ExfatDentry {
    type Error = crate::error::Error;
    fn try_from(dentry: RawExfatDentry) -> Result<Self> {
        let dentry_bytes = dentry.as_bytes();
        match dentry.dentry_type {
            EXFAT_FILE => Ok(ExfatDentry::File(ExfatFileDentry::from_bytes(dentry_bytes))),
            EXFAT_STREAM => Ok(ExfatDentry::Stream(ExfatStreamDentry::from_bytes(
                dentry_bytes,
            ))),
            EXFAT_NAME => Ok(ExfatDentry::Name(ExfatNameDentry::from_bytes(dentry_bytes))),
            EXFAT_BITMAP => Ok(ExfatDentry::Bitmap(ExfatBitmapDentry::from_bytes(
                dentry_bytes,
            ))),
            EXFAT_UPCASE => Ok(ExfatDentry::Upcase(ExfatUpcaseDentry::from_bytes(
                dentry_bytes,
            ))),
            EXFAT_VENDOR_EXT => Ok(ExfatDentry::VendorExt(ExfatVendorExtDentry::from_bytes(
                dentry_bytes,
            ))),
            EXFAT_VENDOR_ALLOC => Ok(ExfatDentry::VendorAlloc(
                ExfatVendorAllocDentry::from_bytes(dentry_bytes),
            )),

            EXFAT_UNUSED => Ok(ExfatDentry::UnUsed),
            // Deleted
            0x01..0x80 => Ok(ExfatDentry::Deleted(ExfatDeletedDentry::from_bytes(
                dentry_bytes,
            ))),
            // Primary
            0x80..0xC0 => Ok(ExfatDentry::GenericPrimary(
                ExfatGenericPrimaryDentry::from_bytes(dentry_bytes),
            )),
            // Secondary
            0xC0..=0xFF => Ok(ExfatDentry::GenericSecondary(
                ExfatGenericSecondaryDentry::from_bytes(dentry_bytes),
            )),
        }
    }
}

// State machine used to validate dentry set.
enum ExfatValidateDentryMode {
    Started,
    GetFile,
    GetStream,
    // 17 name dentires at maximal.
    GetName(usize),
    GetBenignSecondary,
}

impl ExfatValidateDentryMode {
    fn transit_to_next_state(&self, dentry: &ExfatDentry) -> Result<Self> {
        const MAX_NAME_DENTRIES: usize = MAX_NAME_LENGTH / EXFAT_FILE_NAME_LEN;
        match self {
            ExfatValidateDentryMode::Started => {
                if matches!(dentry, ExfatDentry::File(_)) {
                    Ok(ExfatValidateDentryMode::GetFile)
                } else {
                    return_errno_with_message!(Errno::EINVAL, "invalid dentry state machine")
                }
            }
            ExfatValidateDentryMode::GetFile => {
                if matches!(dentry, ExfatDentry::Stream(_)) {
                    Ok(ExfatValidateDentryMode::GetStream)
                } else {
                    return_errno_with_message!(Errno::EINVAL, "invalid dentry state machine")
                }
            }
            ExfatValidateDentryMode::GetStream => {
                if matches!(dentry, ExfatDentry::Name(_)) {
                    Ok(ExfatValidateDentryMode::GetName(0))
                } else {
                    return_errno_with_message!(Errno::EINVAL, "invalid dentry state machine")
                }
            }
            ExfatValidateDentryMode::GetName(count) => {
                if count + 1 < MAX_NAME_DENTRIES && matches!(dentry, ExfatDentry::Name(_)) {
                    Ok(ExfatValidateDentryMode::GetName(count + 1))
                } else if matches!(dentry, ExfatDentry::GenericSecondary(_))
                    || matches!(dentry, ExfatDentry::VendorAlloc(_))
                    || matches!(dentry, ExfatDentry::VendorExt(_))
                {
                    Ok(ExfatValidateDentryMode::GetBenignSecondary)
                } else {
                    return_errno_with_message!(Errno::EINVAL, "invalid dentry state machine")
                }
            }
            ExfatValidateDentryMode::GetBenignSecondary => {
                if matches!(dentry, ExfatDentry::GenericSecondary(_))
                    || matches!(dentry, ExfatDentry::VendorAlloc(_))
                    || matches!(dentry, ExfatDentry::VendorExt(_))
                {
                    Ok(ExfatValidateDentryMode::GetBenignSecondary)
                } else {
                    return_errno_with_message!(Errno::EINVAL, "invalid dentry state machine")
                }
            }
        }
    }
}

pub trait Checksum {
    fn verify_checksum(&self) -> bool;
    fn update_checksum(&mut self);
}

/// A set of dentries that collectively describe a file or folder.
/// Root directory cannot be represented as an ordinal dentryset.
pub(super) struct ExfatDentrySet {
    dentries: Vec<ExfatDentry>,
}

impl ExfatDentrySet {
    /// Entry set indexes
    /// File dentry index.
    const ES_IDX_FILE: usize = 0;
    /// Stream dentry index.
    const ES_IDX_STREAM: usize = 1;
    /// Name dentry index.
    #[allow(dead_code)]
    const ES_IDX_FIRST_FILENAME: usize = 2;

    pub(super) fn new(dentries: Vec<ExfatDentry>, should_checksum_match: bool) -> Result<Self> {
        let mut dentry_set = ExfatDentrySet { dentries };
        if !should_checksum_match {
            dentry_set.update_checksum();
        }
        dentry_set.validate_dentry_set()?;
        Ok(dentry_set)
    }

    pub(super) fn from(
        fs: Arc<ExfatFS>,
        name: &str,
        inode_type: InodeType,
        _mode: InodeMode,
    ) -> Result<Self> {
        let attrs = {
            if inode_type == InodeType::Dir {
                FatAttr::DIRECTORY.bits()
            } else {
                0
            }
        };

        let name = ExfatName::from_str(name, fs.upcase_table())?;
        let mut name_dentries = name.to_dentries();

        let dos_time = DosTimestamp::now()?;

        let mut dentries = Vec::new();
        let file_dentry = ExfatDentry::File(ExfatFileDentry {
            dentry_type: EXFAT_FILE,
            num_secondary: (name_dentries.len() + 1) as u8,
            checksum: 0,
            attribute: attrs,
            reserved1: 0,
            create_utc_offset: dos_time.utc_offset,
            create_date: dos_time.date,
            create_time: dos_time.time,
            create_time_cs: dos_time.increment_10ms,
            modify_utc_offset: dos_time.utc_offset,
            modify_date: dos_time.date,
            modify_time: dos_time.time,
            modify_time_cs: dos_time.increment_10ms,
            access_utc_offset: dos_time.utc_offset,
            access_date: dos_time.date,
            access_time: dos_time.time,
            reserved2: [0; 7],
        });

        let stream_dentry = ExfatDentry::Stream(ExfatStreamDentry {
            dentry_type: EXFAT_STREAM,
            flags: FatChainFlags::FAT_CHAIN_NOT_IN_USE.bits(),
            reserved1: 0,
            name_len: name.0.len() as u8,
            name_hash: name.checksum(),
            reserved2: 0,
            valid_size: 0,
            reserved3: 0,
            start_cluster: 0,
            size: 0,
        });

        dentries.push(file_dentry);
        dentries.push(stream_dentry);
        dentries.append(&mut name_dentries);

        Self::new(dentries, false)
    }

    pub(super) fn read_from(page_cache: Vmo<Full>, offset: usize) -> Result<Self> {
        let mut iter = ExfatDentryIterator::new(page_cache, offset, None)?;
        let primary_dentry_result = iter.next();

        if primary_dentry_result.is_none() {
            return_errno!(Errno::ENOENT)
        }

        let primary_dentry = primary_dentry_result.unwrap()?;

        if let ExfatDentry::File(file_dentry) = primary_dentry {
            Self::read_from_iterator(&file_dentry, &mut iter)
        } else {
            return_errno_with_message!(Errno::EIO, "invalid dentry type, file dentry expected")
        }
    }

    pub(super) fn read_from_iterator(
        file_dentry: &ExfatFileDentry,
        iter: &mut ExfatDentryIterator,
    ) -> Result<Self> {
        let num_secondary = file_dentry.num_secondary as usize;

        let mut dentries = Vec::<ExfatDentry>::with_capacity(num_secondary + 1);
        dentries.push(ExfatDentry::File(*file_dentry));

        for _i in 0..num_secondary {
            let dentry_result = iter.next();
            if dentry_result.is_none() {
                return_errno!(Errno::ENOENT);
            }
            let dentry = dentry_result.unwrap()?;
            dentries.push(dentry);
        }

        Self::new(dentries, true)
    }

    pub(super) fn len(&self) -> usize {
        self.dentries.len()
    }

    pub(super) fn to_le_bytes(&self) -> Vec<u8> {
        // It may be slow to copy at the granularity of byte.
        // self.dentries.iter().map(|dentry| dentry.to_le_bytes()).flatten().collect::<Vec<u8>>()

        let mut bytes = vec![0; self.dentries.len() * DENTRY_SIZE];
        for (i, dentry) in self.dentries.iter().enumerate() {
            let dentry_bytes = dentry.as_le_bytes();
            let (_, to_write) = bytes.split_at_mut(i * DENTRY_SIZE);
            to_write[..DENTRY_SIZE].copy_from_slice(dentry_bytes)
        }
        bytes
    }

    fn validate_dentry_set(&self) -> Result<()> {
        let mut status = ExfatValidateDentryMode::Started;

        // Maximum dentries = 255 + 1(File dentry)
        if self.dentries.len() > u8::MAX as usize + 1 {
            return_errno_with_message!(Errno::EINVAL, "too many dentries")
        }

        for dentry in &self.dentries {
            status = status.transit_to_next_state(dentry)?;
        }

        if !matches!(status, ExfatValidateDentryMode::GetName(_))
            && !matches!(status, ExfatValidateDentryMode::GetBenignSecondary)
        {
            return_errno_with_message!(Errno::EINVAL, "dentries not enough")
        }

        if !self.verify_checksum() {
            return_errno_with_message!(Errno::EINVAL, "checksum mismatched")
        }

        Ok(())
    }

    pub(super) fn get_file_dentry(&self) -> ExfatFileDentry {
        if let ExfatDentry::File(file) = self.dentries[Self::ES_IDX_FILE] {
            file
        } else {
            panic!("Not possible")
        }
    }

    pub(super) fn set_file_dentry(&mut self, file: &ExfatFileDentry) {
        self.dentries[Self::ES_IDX_FILE] = ExfatDentry::File(*file);
    }

    pub(super) fn get_stream_dentry(&self) -> ExfatStreamDentry {
        if let ExfatDentry::Stream(stream) = self.dentries[Self::ES_IDX_STREAM] {
            stream
        } else {
            panic!("Not possible")
        }
    }

    pub(super) fn set_stream_dentry(&mut self, stream: &ExfatStreamDentry) {
        self.dentries[Self::ES_IDX_STREAM] = ExfatDentry::Stream(*stream);
    }

    pub(super) fn get_name(
        &self,
        upcase_table: Arc<SpinLock<ExfatUpcaseTable>>,
    ) -> Result<ExfatName> {
        let name_dentries: Vec<ExfatNameDentry> = self
            .dentries
            .iter()
            .filter_map(|&dentry| {
                if let ExfatDentry::Name(name_dentry) = dentry {
                    Some(name_dentry)
                } else {
                    None
                }
            })
            .collect();

        let name = ExfatName::from_name_dentries(&name_dentries, upcase_table)?;
        if name.checksum() != self.get_stream_dentry().name_hash {
            return_errno_with_message!(Errno::EINVAL, "name hash mismatched")
        }
        Ok(name)
    }

    /// Name dentries are not permitted to modify. We should create a new dentry set for renaming.
    fn calculate_checksum(&self) -> u16 {
        const CHECKSUM_BYTES_RANGE: Range<usize> = 2..4;
        const EMPTY_RANGE: Range<usize> = 0..0;

        let mut checksum = calc_checksum_16(
            self.dentries[Self::ES_IDX_FILE].as_le_bytes(),
            CHECKSUM_BYTES_RANGE,
            0,
        );

        for i in 1..self.dentries.len() {
            let dentry = &self.dentries[i];
            checksum = calc_checksum_16(dentry.as_le_bytes(), EMPTY_RANGE, checksum);
        }
        checksum
    }
}

impl Checksum for ExfatDentrySet {
    fn verify_checksum(&self) -> bool {
        let checksum = self.calculate_checksum();
        let file = self.get_file_dentry();
        file.checksum == checksum
    }

    fn update_checksum(&mut self) {
        let checksum = self.calculate_checksum();
        let mut file = self.get_file_dentry();
        file.checksum = checksum;
        self.dentries[Self::ES_IDX_FILE] = ExfatDentry::File(file);
    }
}

pub(super) struct ExfatDentryIterator {
    /// The dentry position in current inode.
    entry: u32,
    /// The page cache of the iterated inode.
    page_cache: Vmo<Full>,
    /// Remaining size that can be iterated. If none, iterate through the whole cluster chain.
    size: Option<usize>,
}

impl ExfatDentryIterator {
    pub fn new(page_cache: Vmo<Full>, offset: usize, size: Option<usize>) -> Result<Self> {
        if size.is_some() && size.unwrap() % DENTRY_SIZE != 0 {
            return_errno_with_message!(Errno::EINVAL, "remaining size unaligned to dentry size")
        }

        if offset % DENTRY_SIZE != 0 {
            return_errno_with_message!(Errno::EINVAL, "dentry offset unaligned to dentry size")
        }

        Ok(Self {
            entry: (offset / DENTRY_SIZE) as u32,
            page_cache,
            size,
        })
    }
}

impl Iterator for ExfatDentryIterator {
    type Item = Result<ExfatDentry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.entry as usize * DENTRY_SIZE >= self.page_cache.size() {
            return None;
        }

        if self.size.is_some() && self.size.unwrap() == 0 {
            return None;
        }

        let byte_start = self.entry as usize * DENTRY_SIZE;
        let mut dentry_buf = [0u8; DENTRY_SIZE];

        let read_result = self.page_cache.read_bytes(byte_start, &mut dentry_buf);

        if let Err(_e) = read_result {
            return Some(Err(Error::with_message(
                Errno::EIO,
                "Unable to read dentry from page cache.",
            )));
        }

        // The result is always OK.
        let dentry_result = ExfatDentry::try_from(RawExfatDentry::from_bytes(&dentry_buf)).unwrap();

        self.entry += 1;
        if self.size.is_some() {
            self.size = Some(self.size.unwrap() - DENTRY_SIZE);
        }

        Some(Ok(dentry_result))
    }
}

/// On-disk dentry formats
#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
// For files & directories
pub(super) struct ExfatFileDentry {
    pub(super) dentry_type: u8, // 0x85
    // Number of Secondary directory entries.
    // 2 to 18 (1 StreamDentry + rest NameDentry)
    pub(super) num_secondary: u8,
    // Checksum of all directory entries in the given set excluding this field,calculated on file and secondary entries.
    pub(super) checksum: u16,

    // bit0: read-only; bit1: hidden; bit2: system; bit4: directory; bit5: archive
    pub(super) attribute: u16,
    pub(super) reserved1: u16,

    // Create time, however, ctime in unix metadata means ***change time***.
    pub(super) create_time: u16,
    pub(super) create_date: u16,

    pub(super) modify_time: u16,
    pub(super) modify_date: u16,

    // The timestamp for access_time has double seconds granularity.
    pub(super) access_time: u16,
    pub(super) access_date: u16,

    // High precision time in 10ms
    pub(super) create_time_cs: u8,
    pub(super) modify_time_cs: u8,

    // Timezone for various time
    pub(super) create_utc_offset: u8,
    pub(super) modify_utc_offset: u8,
    pub(super) access_utc_offset: u8,

    pub(super) reserved2: [u8; 7],
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
// MUST be immediately follow the FileDentry (the second dentry in a dentry set)
pub(super) struct ExfatStreamDentry {
    pub(super) dentry_type: u8, // 0xC0
    pub(super) flags: u8, // bit0: AllocationPossible (must be 1); bit1: NoFatChain (=1 <=> contiguous)
    pub(super) reserved1: u8,
    pub(super) name_len: u8,   // file name length (in Unicode - 2 bytes)
    pub(super) name_hash: u16, // something like checksum for file name (calculated in bytes)
    pub(super) reserved2: u16,
    pub(super) valid_size: u64, // file current size
    pub(super) reserved3: u32,
    pub(super) start_cluster: u32, // file start cluster
    pub(super) size: u64,          // file maximum size (not used in init a inode?)
}

pub type UTF16Char = u16;

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
// MUST be immediately follow the StreamDentry in the number of NameLength/15 rounded up
pub(super) struct ExfatNameDentry {
    pub(super) dentry_type: u8,                                // 0xC1
    pub(super) flags: u8,                                      // first two bits must be zero
    pub(super) unicode_0_14: [UTF16Char; EXFAT_FILE_NAME_LEN], // 15 (or less) characters of file name
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct ExfatBitmapDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) reserved: [u8; 18],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct ExfatUpcaseDentry {
    pub(super) dentry_type: u8,
    pub(super) reserved1: [u8; 3],
    pub(super) checksum: u32,
    pub(super) reserved2: [u8; 12],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct ExfatVendorExtDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) vendor_guid: [u8; 16],
    pub(super) vendor_defined: [u8; 14],
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct ExfatVendorAllocDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) vendor_guid: [u8; 16],
    pub(super) vendor_defined: [u8; 2],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct ExfatGenericPrimaryDentry {
    pub(super) dentry_type: u8,
    pub(super) secondary_count: u8,
    pub(super) checksum: u16,
    pub(super) flags: u16,
    pub(super) custom_defined: [u8; 14],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct ExfatGenericSecondaryDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) custom_defined: [u8; 18],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub(super) struct ExfatDeletedDentry {
    pub(super) dentry_type: u8,
    pub(super) reserved: [u8; 31],
}

#[derive(Default, Debug)]
pub(super) struct ExfatName(Vec<UTF16Char>);

impl ExfatName {
    pub fn from_name_dentries(
        names: &[ExfatNameDentry],
        upcase_table: Arc<SpinLock<ExfatUpcaseTable>>,
    ) -> Result<Self> {
        let mut exfat_name = ExfatName::new();
        for name in names {
            for value in name.unicode_0_14 {
                if value == 0 {
                    return Ok(exfat_name);
                }
                exfat_name.push_char(value, upcase_table.clone())?;
            }
        }
        Ok(exfat_name)
    }

    fn push_char(
        &mut self,
        value: UTF16Char,
        _upcase_table: Arc<SpinLock<ExfatUpcaseTable>>,
    ) -> Result<()> {
        if !Self::is_valid_char(value) {
            return_errno_with_message!(Errno::EINVAL, "not a valid char")
        }
        self.0.push(value);
        // self.0.push(upcase_table.lock().transform_char_to_upcase(value)?);
        Ok(())
    }

    fn is_valid_char(value: UTF16Char) -> bool {
        match value {
            0..0x20 => false, // Control Code
            0x22 => false,    // Quotation Mark
            0x2A => false,    // Asterisk
            0x2F => false,    // Forward slash
            0x3A => false,    // Colon
            0x3C => false,    // Less-than sign
            0x3E => false,    // Greater-than sign
            0x3F => false,    // Question mark
            0x5C => false,    // Back slash
            0x7C => false,    // Vertical bar
            _ => true,
        }
    }

    pub fn checksum(&self) -> u16 {
        let bytes = self
            .0
            .iter()
            .flat_map(|character| character.to_le_bytes())
            .collect::<Vec<u8>>();
        const EMPTY_RANGE: Range<usize> = 0..0;
        calc_checksum_16(&bytes, EMPTY_RANGE, 0)
    }

    pub fn from_str(name: &str, _upcase_table: Arc<SpinLock<ExfatUpcaseTable>>) -> Result<Self> {
        let name = ExfatName(name.encode_utf16().collect());
        // upcase_table.lock().transform_to_upcase(&mut name.0)?;
        name.verify()?;
        Ok(name)
    }

    pub fn new() -> Self {
        ExfatName(Vec::new())
    }

    pub fn to_dentries(&self) -> Vec<ExfatDentry> {
        let mut name_dentries = Vec::new();
        for start in (0..self.0.len()).step_by(EXFAT_FILE_NAME_LEN) {
            let end = (start + EXFAT_FILE_NAME_LEN).min(self.0.len());
            let mut name: [u16; EXFAT_FILE_NAME_LEN] = [0; EXFAT_FILE_NAME_LEN];

            name[..end - start].copy_from_slice(&self.0[start..end]);

            name_dentries.push(ExfatDentry::Name(ExfatNameDentry {
                dentry_type: EXFAT_NAME,
                flags: 0,
                unicode_0_14: name,
            }))
        }
        name_dentries
    }

    pub(super) fn verify(&self) -> Result<()> {
        if self
            .0
            .iter()
            .any(|&uni_char| !Self::is_valid_char(uni_char))
        {
            return_errno_with_message!(Errno::EINVAL, "invalid file name.")
        }
        // TODO:verify dots
        Ok(())
    }
}

impl Display for ExfatName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", String::from_utf16_lossy(&self.0))
    }
}

impl Clone for ExfatName {
    fn clone(&self) -> Self {
        ExfatName(self.0.clone())
    }
}
