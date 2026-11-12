// SPDX-License-Identifier: MPL-2.0

//! MBR and GPT partition discovery for whole disks.

use alloc::format;

use device_id::{DeviceId, MinorId};
use ostd::mm::VmIo;
use ostd_pod::Pod;

use crate::{
    AnyBlockDevice, BlockDeviceKind, BlockDeviceMeta, DEVICE_MINORS, DEVICE_REGISTRY, Error,
    SECTOR_SIZE,
    bio::{BioEnqueueError, SubmittedBio},
    device_id::EXTENDED_DEVICE_ID_ALLOCATOR,
    prelude::*,
};

/// Represents a partition entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PartitionInfo {
    Mbr(MbrEntry),
    Gpt(GptEntry),
}

impl PartitionInfo {
    fn start_sector(&self) -> u64 {
        match self {
            PartitionInfo::Mbr(entry) => entry.start_sector as u64,
            PartitionInfo::Gpt(entry) => entry.start_lba,
        }
    }

    fn total_sectors(&self) -> u64 {
        match self {
            PartitionInfo::Mbr(entry) => entry.total_sectors as u64,
            PartitionInfo::Gpt(entry) => entry.end_lba - entry.start_lba + 1,
        }
    }
}

/// A MBR (Master Boot Record) partition table header.
///
/// See <https://wiki.osdev.org/MBR_(x86)#MBR_Format>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct MbrHeader {
    bootstrap_code: [u8; 440],
    id: u32,
    reserved: u16,
    entries: [MbrEntry; 4],
    signature: u16,
}

impl MbrHeader {
    fn check_signature(&self) -> bool {
        self.signature == 0xAA55
    }
}

/// A MBR (Master Boot Record) partition entry.
///
/// See <https://wiki.osdev.org/Partition_Table>.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub(super) struct MbrEntry {
    flag: u8,
    start_chs: ChsAddr,
    type_: u8,
    end_chs: ChsAddr,
    start_sector: u32,
    total_sectors: u32,
}

impl MbrEntry {
    fn is_extended(&self) -> bool {
        self.type_ == 0x05 || self.type_ == 0x0F
    }

    fn is_valid(&self) -> bool {
        // A System ID byte value of 0 is the definitive indicator for an unused entry.
        // Any other illegal value (CHS Sector = 0 or Total Sectors = 0) may also indicate an unused entry.
        self.type_ != 0x00
            && self.start_chs.0[1] != 0
            && self.end_chs.0[1] != 0
            && self.total_sectors != 0
    }
}

/// A CHS (Cylinder-Head-Sector) address.
///
/// In CHS addressing, sector numbers always start at 1; there is no sector 0.
///
/// The CHS address is stored as a 3-byte field:
/// - Byte 0: Head number (8 bits)
/// - Byte 1: Bits 0–5 are the sector number (6 bits, valid values 1–63);
///   bits 6–7 are the upper two bits of the cylinder number
/// - Byte 2: Lower 8 bits of the cylinder number (bits 0–7)
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
struct ChsAddr([u8; 3]);

/// A GPT (GUID Partition Table) header.
///
/// See <https://wiki.osdev.org/GPT#LBA_1:_Partition_Table_Header>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct GptHeader {
    signature: u64,
    revision: u32,
    size: u32,
    crc32: u32,
    reserved: u32,
    current_lba: u64,
    backup_lba: u64,
    first_usable_lba: u64,
    last_usable_lba: u64,
    guid: [u8; 16],
    partition_entry_lba: u64,
    nr_partition_entries: u32,
    size_of_partition_entry: u32,
    crc32_of_partition_entries: u32,
    _padding: [u8; 420],
}

impl GptHeader {
    fn check_signature(&self) -> bool {
        &self.signature.to_le_bytes() == b"EFI PART"
    }
}

/// A GPT (GUID Partition Table) partition entry.
///
/// See <https://wiki.osdev.org/GPT#LBA_2:_Partition_Entries>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub(super) struct GptEntry {
    // Unique ID that defines the purpose and type of this Partition.
    // A value of zero defines that this partition entry is not being used.
    type_guid: [u8; 16],
    // GUID that is unique for every partition entry.
    guid: [u8; 16],
    start_lba: u64,
    end_lba: u64,
    attributes: u64,
    // Null-terminated string containing a human-readable name of the partition.
    name: [u8; 72],
}

impl GptEntry {
    fn is_valid(&self) -> bool {
        self.type_guid != [0; 16]
    }
}

fn parse(device: &Arc<dyn AnyBlockDevice>) -> ostd::Result<Vec<Option<PartitionInfo>>> {
    let mbr = device.read_val::<MbrHeader>(0)?;

    // 0xEE indicates a GPT Protective MBR, a fake partition covering the entire disk.
    if mbr.check_signature() && mbr.entries[0].type_ != 0xEE {
        parse_mbr(device, &mbr)
    } else {
        parse_gpt(device)
    }
}

fn parse_mbr(
    device: &Arc<dyn AnyBlockDevice>,
    mbr: &MbrHeader,
) -> ostd::Result<Vec<Option<PartitionInfo>>> {
    let mut partitions = Vec::new();
    let mut extended_partition = None;
    for entry in mbr.entries {
        if entry.is_extended() {
            extended_partition = Some(entry.start_sector);
        }

        if entry.is_valid() {
            partitions.push(Some(PartitionInfo::Mbr(entry)));
        } else {
            partitions.push(None);
        }
    }

    if let Some(start_sector) = extended_partition {
        parse_ebr(device, &mut partitions, start_sector, 0)?;
    }

    Ok(partitions)
}

fn parse_ebr(
    device: &Arc<dyn AnyBlockDevice>,
    partitions: &mut Vec<Option<PartitionInfo>>,
    start_sector: u32,
    offset: u32,
) -> ostd::Result<()> {
    let ebr_sector = start_sector + offset;
    let mut ebr = device.read_val::<MbrHeader>(ebr_sector as usize * SECTOR_SIZE)?;
    if ebr.entries[0].is_valid() {
        ebr.entries[0].start_sector += ebr_sector;
        partitions.push(Some(PartitionInfo::Mbr(ebr.entries[0])));
    }

    if ebr.entries[1].is_extended() {
        parse_ebr(
            device,
            partitions,
            start_sector,
            ebr.entries[1].start_sector,
        )?;
    }

    Ok(())
}

fn parse_gpt(device: &Arc<dyn AnyBlockDevice>) -> ostd::Result<Vec<Option<PartitionInfo>>> {
    let mut partitions = Vec::new();

    // The primary GPT Header must be located in LBA 1.
    let gpt = device.read_val::<GptHeader>(SECTOR_SIZE)?;

    if !gpt.check_signature() {
        return Ok(partitions);
    }

    // TODO: Check the CRC32 of the header and the partition entries, check the backup GPT header.

    let entry_size = gpt.size_of_partition_entry as usize;
    let entries_per_sector = SECTOR_SIZE / entry_size;
    let total_sectors = gpt.nr_partition_entries as usize / entries_per_sector;
    for i in 0..total_sectors {
        let mut buf = [0u8; SECTOR_SIZE];
        let offset = (gpt.partition_entry_lba as usize + i) * SECTOR_SIZE;
        device.read_bytes(offset, buf.as_mut_slice())?;

        for j in 0..entries_per_sector {
            let entry_offset = j * gpt.size_of_partition_entry as usize;
            let entry = GptEntry::from_first_bytes(&buf[entry_offset..entry_offset + entry_size]);
            if entry.is_valid() {
                partitions.push(Some(PartitionInfo::Gpt(entry)));
            } else {
                partitions.push(None);
            }
        }
    }

    Ok(partitions)
}

#[derive(Debug)]
struct PartitionNode {
    id: DeviceId,
    name: String,
    device: Arc<dyn AnyBlockDevice>,
    info: PartitionInfo,
    number: u32,
}

impl AnyBlockDevice for PartitionNode {
    fn enqueue(&self, mut bio: SubmittedBio) -> Result<(), BioEnqueueError> {
        bio.set_sid_offset(self.info.start_sector());
        self.device.enqueue(bio)
    }

    fn metadata(&self) -> BlockDeviceMeta {
        let mut metadata = self.device.metadata();
        metadata.nr_sectors = self.info.total_sectors() as usize;
        metadata
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn kind(&self) -> BlockDeviceKind<'_> {
        BlockDeviceKind::Partition {
            number: self.number,
            start_sector: self.info.start_sector(),
        }
    }
}

impl PartitionNode {
    fn new(
        device: Arc<dyn AnyBlockDevice>,
        info: PartitionInfo,
        number: u32,
    ) -> Result<Self, Error> {
        let id = if number < DEVICE_MINORS {
            let device_id = device.id();
            DeviceId::new(
                device_id.major(),
                MinorId::new(device_id.minor().get() + number),
            )
        } else {
            EXTENDED_DEVICE_ID_ALLOCATOR.get().unwrap().allocate()?
        };

        Ok(Self {
            id,
            name: partition_name(device.name(), number),
            device,
            info,
            number,
        })
    }
}

impl Drop for PartitionNode {
    fn drop(&mut self) {
        if self.number >= DEVICE_MINORS {
            EXTENDED_DEVICE_ID_ALLOCATOR.get().unwrap().release(self.id);
        }
    }
}

/// Scans and registers the initial partitions of a whole disk.
pub(super) fn scan(device: &Arc<dyn AnyBlockDevice>) -> Result<(), Error> {
    let infos = parse(device).map_err(|_| Error::IoError)?;

    let mut new_partitions = Vec::new();
    for (index, info) in infos.into_iter().enumerate() {
        let Some(info) = info else {
            continue;
        };
        let partition = PartitionNode::new(device.clone(), info, index as u32 + 1)?;
        new_partitions.push(Arc::new(partition));
    }

    // Keep registration and rollback under the registry lock so no caller
    // can observe a partially registered partition list.
    let mut registry = DEVICE_REGISTRY.lock();
    for (index, partition) in new_partitions.iter().enumerate() {
        if registry.contains_key(&partition.id().to_raw()) {
            for registered in &new_partitions[..index] {
                registry.remove(&registered.id().to_raw());
            }
            return Err(Error::Registered);
        }
        registry.insert(partition.id().to_raw(), partition.clone());
    }

    Ok(())
}

/// Formats the name of a partition.
///
/// We perform the naming similar to the Linux implementation: insert "p" between
/// the disk name and the partition number when the disk name ends with a digit
/// (`nvme0n1p1`), and append the number otherwise (`vda1`).
///
/// Reference: <https://elixir.bootlin.com/linux/v7.2.2/source/block/partitions/core.c#L337>
fn partition_name(disk_name: &str, partno: u32) -> String {
    if disk_name.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{}p{}", disk_name, partno)
    } else {
        format!("{}{}", disk_name, partno)
    }
}
