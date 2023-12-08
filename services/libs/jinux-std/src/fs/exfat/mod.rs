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

static EXFAT_IMAGE: &[u8] = include_bytes!("../../../../../../exfat.img");

use crate::fs::exfat::{block_device::ExfatMemoryDisk, fs::ExfatMountOptions};
use crate::prelude::*;
use alloc::boxed::Box;
use jinux_frame::vm::{VmAllocOptions, VmIo, VmSegment};

fn new_vm_segment_from_image() -> Result<VmSegment> {
    let vm_segment = VmAllocOptions::new(EXFAT_IMAGE.len() / PAGE_SIZE)
        .is_contiguous(true)
        .alloc_contiguous()?;

    vm_segment.write_bytes(0, EXFAT_IMAGE)?;
    Ok(vm_segment)
}

pub fn load_exfat() -> Arc<ExfatFS> {
    let vm_segment = new_vm_segment_from_image().unwrap();

    let disk = ExfatMemoryDisk::new(vm_segment);
    let mount_option = ExfatMountOptions::default();

    let fs = ExfatFS::open(Box::new(disk), mount_option);

    assert!(fs.is_ok(), "Fs failed to init:{:?}", fs.unwrap_err());

    fs.unwrap()
}

mod test {
    use crate::{
        fs::{
            exfat::block_device::SECTOR_SIZE,
            utils::{Inode, InodeMode},
        },
        prelude::*,
    };

    use super::load_exfat;
    fn create_file(parent: Arc<dyn Inode>, filename: &str) -> Arc<dyn Inode> {
        let create_result = parent.create(
            filename,
            crate::fs::utils::InodeType::File,
            InodeMode::all(),
        );

        assert!(
            create_result.is_ok(),
            "Fs failed to create: {:?}",
            create_result.unwrap_err()
        );

        create_result.unwrap()
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
            create_file(root.clone(), file_name);

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

        //Test skiped readdir.
        let mut sub_inodes: Vec<String> = Vec::new();
        let _ = root.readdir_at(file_names.len() / 3, &mut sub_inodes);

        assert!(sub_inodes.len() == file_names.len() - file_names.len() / 3);
    }

    #[ktest]
    fn test_mkdir() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let folder_name = "sub";
        let create_result = root.create(
            folder_name,
            crate::fs::utils::InodeType::Dir,
            InodeMode::all(),
        );

        assert!(
            create_result.is_ok(),
            "Fs failed to create: {:?}",
            create_result.unwrap_err()
        );

        let mut sub_dirs: Vec<String> = Vec::new();
        let _ = root.readdir_at(0, &mut sub_dirs);
        assert!(sub_dirs.len() == 1);
        assert!(sub_dirs[0] == folder_name);
    }

    #[ktest]
    fn test_write_and_read_file_direct() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let file = create_file(root.clone(), "test");

        const BUF_SIZE: usize = PAGE_SIZE * 7 + 3 * SECTOR_SIZE;

        let mut buf = vec![0u8; BUF_SIZE];
        for (i, num) in buf.iter_mut().enumerate() {
            //Use a prime number to make each sector different.
            *num = (i % 107) as u8;
        }

        let write_result = file.write_direct_at(0, &buf);
        assert!(
            write_result.is_ok(),
            "Fs failed to write direct: {:?}",
            write_result.unwrap_err()
        );

        let mut read = vec![0u8; BUF_SIZE];
        let read_result = file.read_direct_at(0, &mut read);
        assert!(
            read_result.is_ok(),
            "Fs failed to read direct: {:?}",
            read_result.unwrap_err()
        );

        assert!(buf.eq(&read), "File mismatch. Data read result:{:?}", read);
    }

    #[ktest]
    fn test_write_and_read_file() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let file = create_file(root.clone(), "test");

        const BUF_SIZE: usize = PAGE_SIZE * 11 + 2023;

        let mut buf = vec![0u8; BUF_SIZE];
        for (i, num) in buf.iter_mut().enumerate() {
            //Use a prime number to make each sector different.
            *num = (i % 107) as u8;
        }

        let write_result = file.write_at(0, &buf);
        assert!(
            write_result.is_ok(),
            "Fs failed to write: {:?}",
            write_result.unwrap_err()
        );

        let mut read = vec![0u8; BUF_SIZE];
        let read_result = file.read_at(0, &mut read);
        assert!(
            read_result.is_ok(),
            "Fs failed to read: {:?}",
            read_result.unwrap_err()
        );

        assert!(buf.eq(&read), "File mismatch. Data read result:{:?}", read);
    }

    #[ktest]
    fn test_interleaved_write() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let a = create_file(root.clone(), "a");
        let b = create_file(root.clone(), "b");

        const BUF_SIZE: usize = PAGE_SIZE * 11 + 2023;

        let mut buf_a = vec![0u8; BUF_SIZE];
        for (i, num) in buf_a.iter_mut().enumerate() {
            //Use a prime number to make each sector different.
            *num = (i % 107) as u8;
        }

        let mut buf_b = vec![0u8; BUF_SIZE];
        for (i, num) in buf_b.iter_mut().enumerate() {
            //Use a prime number to make each sector different.
            *num = (i % 109) as u8;
        }

        let steps = 7;
        let write_len = (BUF_SIZE + steps - 1) / steps;
        for i in 0..steps {
            let start = i * write_len;
            let end = BUF_SIZE.min(start + write_len);
            a.write_at(start, &buf_a[start..end]).unwrap();
            b.write_at(start, &buf_b[start..end]).unwrap();
        }

        let mut read = vec![0u8; BUF_SIZE];
        a.read_at(0, &mut read).unwrap();
        assert!(
            buf_a.eq(&read),
            "File a mismatch. Data read result:{:?}",
            read
        );

        b.read_at(0, &mut read).unwrap();
        assert!(
            buf_b.eq(&read),
            "File b mismatch. Data read result:{:?}",
            read
        );
    }
}
