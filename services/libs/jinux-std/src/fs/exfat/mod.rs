mod bitmap;
mod block_device;
mod constants;
mod dentry;
mod fat;
mod fs;
mod inode;
mod super_block;
mod upcase_table;
mod utils;

pub use fs::ExfatFS;
pub use inode::ExfatInode;

const FS_SIZE: usize = 1 << 27;

static EXFAT_IMAGE: &[u8; FS_SIZE] = include_bytes!("../../../../../../exfat.img");

mod test {
    use super::{block_device::ExfatMemoryDisk, fs::ExfatMountOptions, *};
    use crate::{
        fs::utils::{Inode, InodeMode},
        prelude::*,
    };
    use alloc::boxed::Box;
    use jinux_frame::vm::{VmAllocOptions, VmIo, VmSegment};

    fn new_vm_segment_from_image() -> Result<VmSegment> {
        let vm_segment = VmAllocOptions::new(FS_SIZE / PAGE_SIZE)
            .is_contiguous(true)
            .alloc_contiguous()?;

        vm_segment.write_bytes(0, EXFAT_IMAGE)?;
        Ok(vm_segment)
    }

    fn load_exfat() -> Arc<ExfatFS> {
        let vm_segment = new_vm_segment_from_image().unwrap();

        let disk = ExfatMemoryDisk::new(vm_segment);
        let mount_option = ExfatMountOptions::default();

        let fs = ExfatFS::open(Box::new(disk), mount_option);

        assert!(fs.is_ok(), "Fs failed to init:{:?}", fs.unwrap_err());

        fs.unwrap()
    }

    #[ktest]
    fn test_new_exfat() {
        load_exfat();
    }

    #[ktest]
    fn test_create_and_list_file() {
        let mut file_names: Vec<String> = (0..100).map(|x| x.to_string().repeat(50)).collect();
        file_names.sort();

        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;

        for (file_id, file_name) in file_names.iter().enumerate() {
            let create_result = root.create(
                file_name,
                crate::fs::utils::InodeType::File,
                InodeMode::all(),
            );

            assert!(
                create_result.is_ok(),
                "Fs failed to create: {:?}",
                create_result.unwrap_err()
            );
            let mut sub_inodes: Vec<String> = Vec::new();

            let read_result = root.readdir_at(0, &mut sub_inodes);
            assert!(
                read_result.is_ok(),
                "Fs failed to readdir: {:?}",
                read_result.unwrap_err()
            );

            assert!(read_result.unwrap() == file_id + 1);
            assert!(sub_inodes.len() == file_id + 1);

            sub_inodes.sort();

            for i in 0..sub_inodes.len() {
                assert!(sub_inodes[i].cmp(&file_names[i]).is_eq())
            }

            info!("Successfully creating and reading {} files", file_id + 1);
        }
    }
}
