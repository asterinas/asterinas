// SPDX-License-Identifier: MPL-2.0

use alloc::format;
use core::sync::atomic::Ordering;

use aster_block::{
    bio::{Bio, BioEnqueueError, BioStatus, SubmittedBio},
    BlockDevice, BlockDeviceMeta, SECTOR_SIZE,
};
use ostd::{mm::VmIo, Pod};

use super::{
    add_node,
    disk::{EXTENDED_MAJOR, EXTENDED_MINOR, VIRTIO_DEVICE_MAJOR, VIRTIO_DEVICE_MINORS},
    DeviceId,
};
use crate::{
    events::IoEvents,
    fs::inode_handle::FileIo,
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Represents a disk partition node (e.g., "/dev/vda1").
#[derive(Debug, Clone)]
pub(super) struct PartitionNode {
    id: DeviceId,
    device: Weak<dyn BlockDevice>,
    partition: Partition,
}

impl PartitionNode {
    pub(super) fn start_sector(&self) -> usize {
        match self.partition {
            Partition::Mbr(m) => m.start_sector as usize,
            Partition::Gpt(g) => g.start_lba as usize,
        }
    }

    pub(super) fn total_sectors(&self) -> usize {
        match self.partition {
            Partition::Mbr(m) => m.total_sectors as usize,
            Partition::Gpt(g) => (g.end_lba - g.start_lba + 1) as usize,
        }
    }
}

impl BlockDevice for PartitionNode {
    fn enqueue(&self, bio: SubmittedBio) -> core::result::Result<(), BioEnqueueError> {
        let Some(device) = self.device.upgrade() else {
            bio.complete(BioStatus::IoError);
            return Ok(());
        };

        let start_sid = bio.sid_range().start + self.start_sector() as u64;
        let segments = Vec::from_iter(bio.segments().iter().cloned());
        let new_bio = Bio::new(bio.type_(), start_sid, segments, None);

        let Ok(status) = new_bio.submit_and_wait(device.as_ref()) else {
            bio.complete(BioStatus::IoError);
            return Ok(());
        };

        bio.complete(status);
        Ok(())
    }

    fn metadata(&self) -> BlockDeviceMeta {
        let mut metadata = BlockDeviceMeta::default();
        let Some(device) = self.device.upgrade() else {
            return metadata;
        };

        metadata.max_nr_segments_per_bio = device.metadata().max_nr_segments_per_bio;
        metadata.nr_sectors = self.total_sectors();
        metadata
    }
}

impl super::Device for PartitionNode {
    fn id(&self) -> DeviceId {
        self.id
    }

    fn type_(&self) -> super::DeviceType {
        super::DeviceType::BlockDevice
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(self.clone())))
    }
}

impl FileIo for PartitionNode {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let Some(device) = self.device.upgrade() else {
            return_errno_with_message!(Errno::EIO, "device is gone");
        };

        let offset = self.start_sector() * SECTOR_SIZE;

        let total = writer.avail();
        device.read(offset, writer)?;
        let avail = writer.avail();
        Ok(total - avail)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let Some(device) = self.device.upgrade() else {
            return_errno_with_message!(Errno::EIO, "device is gone");
        };

        let offset = self.start_sector() * SECTOR_SIZE;

        let total = reader.remain();
        device.write(offset, reader)?;
        let remain = reader.remain();
        Ok(total - remain)
    }
}

impl Pollable for PartitionNode {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

/// Represents a partition entry.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum Partition {
    Mbr(MbrEntry),
    Gpt(GptEntry),
}

/// A MBR (Master Boot Record) partition table header.
///
/// See <https://wiki.osdev.org/MBR_(x86)#MBR_Format>.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
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
#[derive(Debug, Copy, Clone, PartialEq, Eq, Pod)]
struct MbrEntry {
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
#[derive(Debug, Copy, Clone, PartialEq, Eq, Pod)]
struct ChsAddr([u8; 3]);

/// A GPT (GUID Partition Table) header.
///
/// See <https://wiki.osdev.org/GPT#LBA_1:_Partition_Table_Header>.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
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
#[derive(Debug, Copy, Clone, PartialEq, Eq, Pod)]
struct GptEntry {
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

pub(super) fn parse_partitions(
    device: &Arc<dyn BlockDevice>,
    id: &DeviceId,
    name: &String,
) -> Vec<Option<Arc<PartitionNode>>> {
    let mbr = device.read_val::<MbrHeader>(0).unwrap();

    // 0xEE indicates a GPT Protective MBR, a fake partition covering the entire disk.
    let partitions = if mbr.check_signature() && mbr.entries[0].type_ != 0xEE {
        parse_mbr_partitions(device, &mbr)
    } else {
        parse_gpt_partitions(device)
    };

    partitions
        .into_iter()
        .enumerate()
        .map(|(i, p)| add_partition_node(device, p, i, id, name))
        .collect()
}

fn parse_mbr_partitions(device: &Arc<dyn BlockDevice>, mbr: &MbrHeader) -> Vec<Option<Partition>> {
    let mut partitions = Vec::new();
    let mut extended_partition = None;
    for entry in mbr.entries {
        if entry.is_extended() {
            extended_partition = Some(entry.start_sector);
        }

        if entry.is_valid() {
            partitions.push(Some(Partition::Mbr(entry)));
        } else {
            partitions.push(None);
        }
    }

    if let Some(start_sector) = extended_partition {
        parse_ebr_partitions(device, &mut partitions, start_sector, 0);
    }

    partitions
}

fn parse_ebr_partitions(
    device: &Arc<dyn BlockDevice>,
    partitions: &mut Vec<Option<Partition>>,
    start_sector: u32,
    offset: u32,
) {
    let ebr_sector = start_sector + offset;
    let mut ebr = device
        .read_val::<MbrHeader>(ebr_sector as usize * SECTOR_SIZE)
        .unwrap();
    if ebr.entries[0].is_valid() {
        ebr.entries[0].start_sector += ebr_sector;
        partitions.push(Some(Partition::Mbr(ebr.entries[0])));
    }

    if ebr.entries[1].is_extended() {
        parse_ebr_partitions(
            device,
            partitions,
            start_sector,
            ebr.entries[1].start_sector,
        );
    }
}

fn parse_gpt_partitions(device: &Arc<dyn BlockDevice>) -> Vec<Option<Partition>> {
    let mut partitions = Vec::new();

    // The primary GPT Header must be located in LBA 1.
    let gpt = device.read_val::<GptHeader>(SECTOR_SIZE).unwrap();

    if !gpt.check_signature() {
        return partitions;
    }

    // TODO: Check the CRC32 of the header and the partition entries, check the backup GPT header.

    let entry_size = gpt.size_of_partition_entry as usize;
    let entries_per_sector = SECTOR_SIZE / entry_size;
    let total_sectors = gpt.nr_partition_entries as usize / entries_per_sector;
    for i in 0..total_sectors {
        let mut buf = [0u8; SECTOR_SIZE];
        let offset = (gpt.partition_entry_lba as usize + i) * SECTOR_SIZE;
        device.read_bytes(offset, buf.as_mut_slice()).unwrap();

        for j in 0..entries_per_sector {
            let entry_offset = j * gpt.size_of_partition_entry as usize;
            let entry = GptEntry::from_bytes(&buf[entry_offset..entry_offset + entry_size]);
            if entry.is_valid() {
                partitions.push(Some(Partition::Gpt(entry)));
            } else {
                partitions.push(None);
            }
        }
    }

    partitions
}

fn add_partition_node(
    device: &Arc<dyn BlockDevice>,
    partition: Option<Partition>,
    index: usize,
    base_id: &DeviceId,
    base_name: &String,
) -> Option<Arc<PartitionNode>> {
    let partition = partition?;

    let (major, minor) = if (index as u32) < VIRTIO_DEVICE_MINORS {
        (VIRTIO_DEVICE_MAJOR, index as u32 + 1)
    } else {
        (
            EXTENDED_MAJOR,
            EXTENDED_MINOR.fetch_add(1, Ordering::Relaxed),
        )
    };

    let id = DeviceId::new(major, minor + base_id.minor());
    let name = format!("{}{}", base_name, index + 1);
    let partition_node = Arc::new(PartitionNode {
        id,
        device: Arc::downgrade(device),
        partition,
    });
    let _ = add_node(partition_node.clone(), name.as_str());
    Some(partition_node)
}
