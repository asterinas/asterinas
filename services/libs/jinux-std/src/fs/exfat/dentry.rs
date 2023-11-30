use core::ops::Range;

use align_ext::AlignExt;
use jinux_frame::vm::{VmAllocOptions, VmFrame, VmFrameVec, VmIo};

use crate::fs::utils::{InodeMode, InodeType};
use crate::prelude::*;
use crate::time::now_as_duration;

use super::constants::{EXFAT_FILE_NAME_LEN, MAX_NAME_LENGTH};
use super::fat::ExfatChainPosition;
use super::fat::{ExfatChain, FatChainFlags};
use super::fs::ExfatFS;
use super::inode::FatAttr;
use super::utils::{calc_checksum_16, DosTimestamp};

pub const DENTRY_SIZE: usize = 32; // directory entry size
#[derive(Debug)]
pub enum ExfatDentry {
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
    fn to_le_bytes(&self) -> &[u8] {
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

    fn read_from(pos: &ExfatChainPosition) -> Result<ExfatDentry> {
        let mut buf = [0u8; DENTRY_SIZE];
        pos.0.read_at(pos.1, &mut buf)?;
        ExfatDentry::try_from(buf.as_bytes())
    }
}

//TODO: Use enums instead of const variables.
// dentry types
const EXFAT_UNUSED: u8 = 0x00; // end of directory

const EXFAT_INVAL: u8 = 0x80; // invalid value
const EXFAT_BITMAP: u8 = 0x81; // allocation bitmap
const EXFAT_UPCASE: u8 = 0x82; // upcase table
const EXFAT_VOLUME: u8 = 0x83; // volume label
const EXFAT_FILE: u8 = 0x85; // file or dir

const EXFAT_GUID: u8 = 0xA0; //GUID of the volume, can be ignored.
const EXFAT_PADDING: u8 = 0xA1; // Can be ignored
const EXFAT_ACLTAB: u8 = 0xA2; // not specified in specification, can be used to provide acl control.

const EXFAT_STREAM: u8 = 0xC0; // stream entry
const EXFAT_NAME: u8 = 0xC1; // file name entry
const EXFAT_ACL: u8 = 0xC2; // not specified in specification, can be used to provide acl control.

const EXFAT_VENDOR_EXT: u8 = 0xE0; // vendor extension entry
const EXFAT_VENDOR_ALLOC: u8 = 0xE1; // vendor allocation entry

impl TryFrom<&[u8]> for ExfatDentry {
    type Error = crate::error::Error;
    fn try_from(value: &[u8]) -> Result<Self> {
        if value.len() != DENTRY_SIZE {
            return_errno_with_message!(Errno::EINVAL, "directory entry size mismatch.")
        }
        match value[0] {
            EXFAT_FILE => Ok(ExfatDentry::File(ExfatFileDentry::from_bytes(value))),
            EXFAT_STREAM => Ok(ExfatDentry::Stream(ExfatStreamDentry::from_bytes(value))),
            EXFAT_NAME => Ok(ExfatDentry::Name(ExfatNameDentry::from_bytes(value))),
            EXFAT_BITMAP => Ok(ExfatDentry::Bitmap(ExfatBitmapDentry::from_bytes(value))),
            EXFAT_UPCASE => Ok(ExfatDentry::Upcase(ExfatUpcaseDentry::from_bytes(value))),
            EXFAT_VENDOR_EXT => Ok(ExfatDentry::VendorExt(ExfatVendorExtDentry::from_bytes(
                value,
            ))),
            EXFAT_VENDOR_ALLOC => Ok(ExfatDentry::VendorAlloc(
                ExfatVendorAllocDentry::from_bytes(value),
            )),

            EXFAT_UNUSED => Ok(ExfatDentry::UnUsed),
            //Deleted
            0x01..0x80 => Ok(ExfatDentry::Deleted(ExfatDeletedDentry::from_bytes(value))),
            //Primary
            0x80..0xC0 => Ok(ExfatDentry::GenericPrimary(
                ExfatGenericPrimaryDentry::from_bytes(value),
            )),
            //Secondary
            0xC0..=0xFF => Ok(ExfatDentry::GenericSecondary(
                ExfatGenericSecondaryDentry::from_bytes(value),
            )),
        }
    }
}

const MAX_NAME_DENTRIES: usize = MAX_NAME_LENGTH / EXFAT_FILE_NAME_LEN;

//State machine used to validate dentry set.
enum ExfatValidateDentryMode {
    Started,
    GetFile,
    GetStream,
    //17 name dentires at maximal.
    GetName(usize),
    GetBenignSecondary,
}

impl ExfatValidateDentryMode {
    fn transit_to_next_state(&self, dentry: &ExfatDentry) -> Result<Self> {
        match self {
            ExfatValidateDentryMode::Started => {
                if matches!(dentry, ExfatDentry::File(_)) {
                    Ok(ExfatValidateDentryMode::GetFile)
                } else {
                    return_errno!(Errno::EINVAL)
                }
            }
            ExfatValidateDentryMode::GetFile => {
                if matches!(dentry, ExfatDentry::Stream(_)) {
                    Ok(ExfatValidateDentryMode::GetStream)
                } else {
                    return_errno!(Errno::EINVAL)
                }
            }
            ExfatValidateDentryMode::GetStream => {
                if matches!(dentry, ExfatDentry::Name(_)) {
                    Ok(ExfatValidateDentryMode::GetName(0))
                } else {
                    return_errno!(Errno::EINVAL)
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
                    return_errno!(Errno::EINVAL)
                }
            }
            ExfatValidateDentryMode::GetBenignSecondary => {
                if matches!(dentry, ExfatDentry::GenericSecondary(_))
                    || matches!(dentry, ExfatDentry::VendorAlloc(_))
                    || matches!(dentry, ExfatDentry::VendorExt(_))
                {
                    Ok(ExfatValidateDentryMode::GetBenignSecondary)
                } else {
                    return_errno!(Errno::EINVAL)
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

pub struct ExfatDentrySet {
    dentries: Vec<ExfatDentry>,
}

impl ExfatDentrySet {
    /// Entry set indexes
    /// File dentry index.
    const ES_IDX_FILE: usize = 0;
    /// Stream dentry index.
    const ES_IDX_STREAM: usize = 1;
    /// Name dentry index.
    const ES_IDX_FIRST_FILENAME: usize = 2;

    pub(super) fn new(dentries: Vec<ExfatDentry>) -> Result<Self> {
        let dentry_set = ExfatDentrySet { dentries };
        dentry_set.validate_dentry_set()?;
        Ok(dentry_set)
    }

    pub(super) fn from(
        fs: Arc<ExfatFS>,
        name: &str,
        inode_type: InodeType,
        mode: InodeMode,
    ) -> Result<Self> {
        let attrs = {
            if inode_type == InodeType::Dir {
                FatAttr::DIRECTORY.bits()
            } else {
                0
            }
        };

        let name = ExfatName::from_str(name)?;
        let mut name_dentries = name.to_dentries();

        let current_time = now_as_duration(&crate::time::ClockID::CLOCK_REALTIME)?;
        let dos_time = DosTimestamp::from_duration(current_time)?;

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
            create_time_cs: dos_time.increament_10ms,
            modify_utc_offset: dos_time.utc_offset,
            modify_date: dos_time.date,
            modify_time: dos_time.time,
            modify_time_cs: dos_time.increament_10ms,
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

        ExfatDentrySet::new(dentries)
    }

    pub(super) fn read_from(pos: &ExfatChainPosition) -> Result<Self> {
        let primary_dentry = ExfatDentry::read_from(pos)?;

        if let ExfatDentry::File(file_dentry) = primary_dentry {
            let num_secondary = file_dentry.num_secondary as usize;

            let mut dentries = Vec::<ExfatDentry>::with_capacity(num_secondary + 1);
            dentries.push(primary_dentry);

            let mut dentry_bytes = Vec::<u8>::with_capacity(num_secondary * DENTRY_SIZE);

            pos.0.read_at(pos.1 + DENTRY_SIZE, &mut dentry_bytes)?;

            for i in 0..num_secondary {
                let dentry =
                    ExfatDentry::try_from(&dentry_bytes[i * DENTRY_SIZE..(i + 1) * DENTRY_SIZE])?;
                dentries.push(dentry);
            }

            Self::new(dentries)
        } else {
            return_errno_with_message!(Errno::EIO, "invalid dentry type")
        }
    }

    pub(super) fn write_at(&self, pos: &ExfatChainPosition) -> Result<usize> {
        let bytes = self.to_le_bytes();
        pos.0.write_at(pos.1, &bytes)
    }

    pub(super) fn len(&self) -> usize {
        self.dentries.len() * DENTRY_SIZE
    }

    fn to_le_bytes(&self) -> Vec<u8> {
        // It may be slow to copy at the granularity of byte.
        //self.dentries.iter().map(|dentry| dentry.to_le_bytes()).flatten().collect::<Vec<u8>>()

        let mut bytes = Vec::<u8>::with_capacity(self.dentries.len() * DENTRY_SIZE);
        for (i, dentry) in self.dentries.iter().enumerate() {
            let dentry_bytes = dentry.to_le_bytes();
            let (_, to_write) = bytes.split_at_mut(i * DENTRY_SIZE);
            to_write[..DENTRY_SIZE].copy_from_slice(dentry_bytes)
        }
        bytes
    }

    fn validate_dentry_set(&self) -> Result<()> {
        let mut status = ExfatValidateDentryMode::Started;

        //Maximum dentries = 255 + 1(File dentry)
        if self.dentries.len() > u8::MAX as usize + 1 {
            return_errno_with_message!(Errno::EINVAL, "too many dentries")
        }

        for dentry in &self.dentries {
            status = status.transit_to_next_state(dentry)?;
        }

        if !matches!(status, ExfatValidateDentryMode::GetName(_))
            || !matches!(status, ExfatValidateDentryMode::GetBenignSecondary)
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

    pub(super) fn get_name(&self) -> Result<ExfatName> {
        let mut name: ExfatName = ExfatName::new();
        for i in Self::ES_IDX_FIRST_FILENAME..self.dentries.len() {
            if let ExfatDentry::Name(name_dentry) = self.dentries[i] {
                for character in name_dentry.unicode_0_14 {
                    if character == 0 {
                        break;
                    } else {
                        name.push(character)?;
                    }
                }
            } else {
                //End of name dentry
                break;
            }
        }
        name.verify()?;
        if name.checksum() != self.get_stream_dentry().name_hash {
            return_errno_with_message!(Errno::EINVAL, "name hash mismatched")
        }
        Ok(name)
    }
    ///Name dentries are not permited to modify. We should create a new dentry set for renaming.

    fn calculate_checksum(&self) -> u16 {
        const CHECKSUM_BYTES_RANGE: Range<usize> = 2..4;
        const EMPTY_RANGE: Range<usize> = 0..0;

        let mut checksum = calc_checksum_16(
            self.dentries[Self::ES_IDX_FILE].to_le_bytes(),
            CHECKSUM_BYTES_RANGE,
            0,
        );

        for i in 1..self.dentries.len() {
            let dentry = &self.dentries[i];
            checksum = calc_checksum_16(dentry.to_le_bytes(), EMPTY_RANGE, checksum);
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

/* How can I implement checksum?
pub enum ObjectWithChecksum<T:Checksum>  {
    ChecksumMatched(T),
    ChecksumUnmatched(T)
}

impl<T> ObjectWithChecksum<T:Checksum> {
    pub fn verify(&self) -> ObjectWithChecksum<T>{
        match self {
            ObjectWithChecksum::ChecksumUnMatched(t) => {if t.verify_checksum() {ObjectWithChecksum::ChecksumMatched(t)} else {self}},
            _ => self
        }
    }

    pub fn is_checksum_matched(&self) -> bool{
        matches!(self,ObjectWithChecksum::ChecksumMatched)
    }

    pub fn update(&mut self) -> ObjectWithChecksum::ChecksumMatched {
        match self {
            ObjectWithChecksum::ChecksumUnMatched(t) => {t.update_checksum(); ObjectWithChecksum::ChecksumMatched(t)},
            _ => self
        }
    }
}
*/

pub struct ExfatDentryIterator {
    ///The position of current cluster
    chain: ExfatChain,
    ///The dentry position in current cluster.
    entry: u32,
    ///Used to hold cached dentries
    buffer: VmFrame,

    ///Remaining size that can be iterated. If none, iterate through the whole cluster chain.
    size: Option<usize>,

    has_error: bool,
    previous_error: Option<Error>,
}

impl ExfatDentryIterator {
    pub fn new(chain: ExfatChain, offset: usize, size: Option<usize>) -> Result<Self> {
        if size.is_some() && size.unwrap() % DENTRY_SIZE != 0 {
            return_errno!(Errno::EINVAL)
        }

        if offset % DENTRY_SIZE != 0 {
            return_errno!(Errno::EINVAL)
        }

        let buffer = VmFrameVec::allocate(VmAllocOptions::new(1).uninit(false).can_dma(true))?
            .pop()
            .unwrap();
        chain.read_page((offset).align_down(PAGE_SIZE), &buffer)?;

        Ok(Self {
            chain,
            entry: (offset / DENTRY_SIZE) as u32,
            buffer,
            size,
            has_error: false,
            previous_error: None,
        })
    }

    pub fn chain_and_offset(&self) -> ExfatChainPosition {
        (self.chain.clone(), self.entry as usize * DENTRY_SIZE)
    }

    fn read_next_page(&mut self) -> Result<()> {
        if self.entry as usize * DENTRY_SIZE == self.chain.cluster_size() {
            self.chain = self.chain.walk(1)?;
            self.entry = 0;
        }
        self.chain.read_page(
            (self.entry as usize * DENTRY_SIZE).align_down(PAGE_SIZE),
            &self.buffer,
        )?;
        Ok(())
    }
}

impl Iterator for ExfatDentryIterator {
    type Item = Result<ExfatDentry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.has_error {
            if self.previous_error.is_some() {
                //TODO:Can be optimized
                let next_eof = self.chain.is_next_cluster_eof();
                if next_eof.is_ok() && next_eof.unwrap() {
                    return None;
                }

                let result = self.previous_error.clone().unwrap();
                return Some(Err(result));
            }
            return None;
        }

        if self.size.is_some() && self.size.unwrap() == 0 {
            return None;
        }

        let byte_start = self.entry as usize * DENTRY_SIZE % PAGE_SIZE;
        let mut dentry_buf = [0u8; DENTRY_SIZE];

        //There will be no errors for reading from page.
        //TODO: is the bytes copy neccessary?
        self.buffer.read_bytes(byte_start, &mut dentry_buf).unwrap();

        let dentry_result = ExfatDentry::try_from(dentry_buf.as_bytes());

        if dentry_result.is_err() {
            self.has_error = true;
            return Some(Err(dentry_result.unwrap_err()));
        }

        self.entry += 1;
        if self.size.is_some() {
            self.size = Some(self.size.unwrap() - DENTRY_SIZE);
        }

        //Read next page if we reach the page boundary
        if (self.size.is_none() || self.size.unwrap() != 0)
            && self.entry as usize * DENTRY_SIZE % PAGE_SIZE == 0
        {
            let load_page_result = self.read_next_page();
            if load_page_result.is_err() {
                self.has_error = true;
                self.previous_error = Some(load_page_result.unwrap_err());
            }
        }

        Some(Ok(dentry_result.unwrap()))
    }
}

/// On-disk dentry formats

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
// For files & directorys
pub struct ExfatFileDentry {
    pub(super) dentry_type: u8, // 0x85
    //Number of Secondary directory entries.
    //2 to 18 (1 StreamDentry + rest NameDentry)
    pub(super) num_secondary: u8,
    // checksum of all directory entries in the given set excluding this field,calculated on file and secondary entries.
    pub(super) checksum: u16,

    // bit0: read-only; bit1: hidden; bit2: system; bit4: directory; bit5: archive
    pub(super) attribute: u16,
    pub(super) reserved1: u16,

    //Create time, however, ctime in unix metadata means ***change time***.
    pub(super) create_time: u16,
    pub(super) create_date: u16,

    pub(super) modify_time: u16,
    pub(super) modify_date: u16,

    //The timestamp for access_time has double seconds granularity.
    pub(super) access_time: u16,
    pub(super) access_date: u16,

    //High precision time in 10ms
    pub(super) create_time_cs: u8,
    pub(super) modify_time_cs: u8,

    //Timezone for various time
    pub(super) create_utc_offset: u8,
    pub(super) modify_utc_offset: u8,
    pub(super) access_utc_offset: u8,

    pub(super) reserved2: [u8; 7],
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
// MUST be immediately follow the FileDentry (the second dentry in a dentry set)
pub struct ExfatStreamDentry {
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

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
// MUST be immediately follow the StreamDentry in the number of NameLength/15 rounded up
pub struct ExfatNameDentry {
    pub(super) dentry_type: u8,                          // 0xC1
    pub(super) flags: u8,                                // first two bits must be zero
    pub(super) unicode_0_14: [u16; EXFAT_FILE_NAME_LEN], // 15 (or less) characters of file name
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub struct ExfatBitmapDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) reserved: [u8; 18],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub struct ExfatUpcaseDentry {
    pub(super) dentry_type: u8,
    pub(super) reserved1: [u8; 3],
    pub(super) checksum: u32,
    pub(super) reserved2: [u8; 12],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub struct ExfatVendorExtDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) vendor_guid: [u8; 16],
    pub(super) vendor_defined: [u8; 14],
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub struct ExfatVendorAllocDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) vendor_guid: [u8; 16],
    pub(super) vendor_defined: [u8; 2],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub struct ExfatGenericPrimaryDentry {
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
pub struct ExfatGenericSecondaryDentry {
    pub(super) dentry_type: u8,
    pub(super) flags: u8,
    pub(super) custom_defined: [u8; 18],
    pub(super) start_cluster: u32,
    pub(super) size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Debug, Default, Copy, Pod)]
pub struct ExfatDeletedDentry {
    pub(super) dentry_type: u8,
    pub(super) reserverd: [u8; 31],
}

#[derive(Default, Debug)]
pub struct ExfatName(Vec<u16>);

impl ExfatName {
    pub(super) fn push(&mut self, value: u16) -> Result<()> {
        if !Self::is_valid_char(value) {
            return_errno_with_message!(Errno::EINVAL, "not a valid char")
        }
        //TODO: should use upcase table.
        self.0.push(value);
        Ok(())
    }

    fn is_valid_char(value: u16) -> bool {
        match value {
            0..0x20 => false, //Control Code
            0x22 => false,    //Quotation Mark
            0x2A => false,    //Asterisk
            0x2F => false,    //Forward slash
            0x3A => false,    //Colon
            0x3C => false,    //Less-than sign
            0x3E => false,    //Greater-than sign
            0x3F => false,    //Question mark
            0x5C => false,    //Back slash
            0x7C => false,    //Vertical bar
            _ => true,
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
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

    pub fn from_str(name: &str) -> Result<Self> {
        //TODO: should use upcase table.
        let name = ExfatName(name.encode_utf16().collect());
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
            name.copy_from_slice(&self.0[start..end]);
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
        //TODO:verify dots
        Ok(())
    }
}

impl ToString for ExfatName {
    fn to_string(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }
}
