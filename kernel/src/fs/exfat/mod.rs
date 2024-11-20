// SPDX-License-Identifier: MPL-2.0

mod bitmap;
mod constants;
mod dentry;
mod fat;
mod fs;
mod inode;
mod super_block;
mod upcase_table;
mod utils;

pub use fs::{ExfatFS, ExfatMountOptions};
pub use inode::ExfatInode;

#[cfg(ktest)]
mod test {
    use alloc::fmt::Debug;

    use aster_block::{
        bio::{BioEnqueueError, BioStatus, BioType, SubmittedBio},
        BlockDevice, BlockDeviceMeta,
    };
    use ostd::{
        mm::{FrameAllocOptions, Segment, VmIo, PAGE_SIZE},
        prelude::*,
    };
    use rand::{rngs::SmallRng, RngCore, SeedableRng};

    use crate::{
        fs::{
            exfat::{
                constants::{EXFAT_RESERVED_CLUSTERS, MAX_NAME_LENGTH},
                ExfatFS, ExfatMountOptions,
            },
            utils::{generate_random_operation, new_fs_in_memory, Inode, InodeMode, InodeType},
        },
        prelude::*,
    };

    /// Followings are implementations of memory simulated block device
    pub const SECTOR_SIZE: usize = 512;
    struct ExfatMemoryBioQueue(Segment);

    impl ExfatMemoryBioQueue {
        pub fn new(segment: Segment) -> Self {
            ExfatMemoryBioQueue(segment)
        }

        pub fn sectors_count(&self) -> usize {
            self.0.nbytes() / SECTOR_SIZE
        }
    }

    pub struct ExfatMemoryDisk {
        queue: ExfatMemoryBioQueue,
    }

    impl ExfatMemoryDisk {
        pub fn new(segment: Segment) -> Self {
            ExfatMemoryDisk {
                queue: ExfatMemoryBioQueue::new(segment),
            }
        }

        pub fn sectors_count(&self) -> usize {
            self.queue.sectors_count()
        }
    }

    impl Debug for ExfatMemoryDisk {
        fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            f.debug_struct("ExfatMemoryDisk")
                .field("blocks_count", &self.sectors_count())
                .finish()
        }
    }

    impl BlockDevice for ExfatMemoryDisk {
        fn enqueue(&self, bio: SubmittedBio) -> core::prelude::v1::Result<(), BioEnqueueError> {
            let start_device_ofs = bio.sid_range().start.to_raw() as usize * SECTOR_SIZE;
            let mut cur_device_ofs = start_device_ofs;
            for seg in bio.segments() {
                let size = match bio.type_() {
                    BioType::Read => seg
                        .inner_segment()
                        .writer()
                        .write(&mut self.queue.0.reader().skip(cur_device_ofs)),
                    BioType::Write => self
                        .queue
                        .0
                        .writer()
                        .skip(cur_device_ofs)
                        .write(&mut seg.inner_segment().reader()),
                    _ => 0,
                };
                cur_device_ofs += size;
            }
            bio.complete(BioStatus::Complete);
            Ok(())
        }

        fn metadata(&self) -> BlockDeviceMeta {
            BlockDeviceMeta {
                max_nr_segments_per_bio: usize::MAX,
                nr_sectors: self.sectors_count(),
            }
        }
    }
    /// Exfat disk image
    static EXFAT_IMAGE: &[u8] = include_bytes!("../../../../test/build/exfat.img");

    /// Read exfat disk image
    fn new_vm_segment_from_image() -> Segment {
        let vm_segment = FrameAllocOptions::new(EXFAT_IMAGE.len().div_ceil(PAGE_SIZE))
            .uninit(true)
            .alloc_contiguous()
            .unwrap();

        vm_segment.write_bytes(0, EXFAT_IMAGE).unwrap();
        vm_segment
    }

    // Generate a simulated exfat file system
    fn load_exfat() -> Arc<ExfatFS> {
        let vm_segment = new_vm_segment_from_image();
        let disk = ExfatMemoryDisk::new(vm_segment);
        let mount_option = ExfatMountOptions::default();
        let fs = ExfatFS::open(Arc::new(disk), mount_option);
        assert!(fs.is_ok(), "Fs failed to init:{:?}", fs.unwrap_err());
        fs.unwrap()
    }

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

    fn create_folder(parent: Arc<dyn Inode>, foldername: &str) -> Arc<dyn Inode> {
        let create_result = parent.create(
            foldername,
            crate::fs::utils::InodeType::Dir,
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
    fn new_exfat() {
        load_exfat();
    }

    #[ktest]
    fn create() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;

        // test basic create
        let file_name = "a.txt";
        create_file(root.clone(), file_name);
        let dir_name = "b";
        create_folder(root.clone(), dir_name);

        // test create with an exist name
        let create_file_with_an_exist_name = root.create(
            dir_name,
            crate::fs::utils::InodeType::File,
            InodeMode::all(),
        );
        let create_dir_with_an_exist_name = root.create(
            file_name,
            crate::fs::utils::InodeType::Dir,
            InodeMode::all(),
        );
        assert!(
            create_dir_with_an_exist_name.is_err() && create_file_with_an_exist_name.is_err(),
            "Fs deal with create an exist name incorrectly"
        );

        // test create with a long name
        let long_file_name = "x".repeat(MAX_NAME_LENGTH);
        let create_long_name_file = root.create(
            &long_file_name,
            crate::fs::utils::InodeType::File,
            InodeMode::all(),
        );
        assert!(
            create_long_name_file.is_ok(),
            "Fail to create a long name file"
        );

        let long_dir_name = "y".repeat(MAX_NAME_LENGTH);
        let create_long_name_dir = root.create(
            &long_dir_name,
            crate::fs::utils::InodeType::Dir,
            InodeMode::all(),
        );
        assert!(
            create_long_name_dir.is_ok(),
            "Fail to create a long name directory"
        );
    }

    #[ktest]
    fn create_and_list_file() {
        let mut file_names: Vec<String> = (0..20).map(|x| x.to_string().repeat(10)).collect();
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

            assert!(read_result.unwrap() == file_id + 1 + 2);
            assert!(sub_inodes.len() == file_id + 1 + 2);

            //Remove . and ..
            sub_inodes.remove(0);
            sub_inodes.remove(0);

            sub_inodes.sort();

            for i in 0..sub_inodes.len() {
                assert!(
                    sub_inodes[i].cmp(&file_names[i]).is_eq(),
                    "i:{:?} Readdir Result:{:?} Filenames:{:?}",
                    i,
                    sub_inodes[i],
                    file_names[i]
                )
            }

            info!("Successfully creating and reading {} files", file_id + 1);
        }

        //Test skipped readdir.
        let mut sub_inodes: Vec<String> = Vec::new();
        let _ = root.readdir_at(file_names.len() / 3 + 2, &mut sub_inodes);

        assert!(sub_inodes.len() == file_names.len() - file_names.len() / 3);
    }

    #[ktest]
    fn unlink_single_file() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let file_name = "a.txt";
        let a_inode = create_file(root.clone(), file_name);
        let _ = a_inode.write_bytes_at(8192, &[0, 1, 2, 3, 4]);

        let unlink_result = root.unlink(file_name);
        assert!(
            unlink_result.is_ok(),
            "Fs failed to unlink: {:?}",
            unlink_result.unwrap_err()
        );

        let mut sub_dirs: Vec<String> = Vec::new();
        let _ = root.readdir_at(0, &mut sub_dirs);

        assert!(sub_dirs.len() == 2);

        // followings are some invalid unlink call. These should return with an error.
        let unlink_fail_result1 = root.unlink(".");
        assert!(
            unlink_fail_result1.is_err(),
            "Fs deal with unlink(.) incorrectly"
        );

        let unlink_fail_result2 = root.unlink("..");
        assert!(
            unlink_fail_result2.is_err(),
            "Fs deal with unlink(..) incorrectly"
        );

        let folder_name = "sub";
        create_folder(root.clone(), folder_name);
        let unlink_dir = root.unlink(folder_name);
        assert!(
            unlink_dir.is_err(),
            "Fs deal with unlink a folder incorrectly"
        );

        // test unlink a long name file
        let long_file_name = "x".repeat(MAX_NAME_LENGTH);
        create_file(root.clone(), &long_file_name);
        let unlink_long_name_file = root.unlink(&long_file_name);
        assert!(
            unlink_long_name_file.is_ok(),
            "Fail to unlink a long name file"
        );
    }

    #[ktest]
    fn unlink_multiple_files() {
        let file_num: u32 = 30; // This shouldn't be too large, better not allocate new clusters for root dir
        let mut file_names: Vec<String> = (0..file_num).map(|x| x.to_string()).collect();
        file_names.sort();

        let fs = load_exfat();
        let cluster_size = fs.cluster_size();
        let root = fs.root_inode() as Arc<dyn Inode>;
        //let mut free_clusters_before_create: Vec<u32> = Vec::new();
        for (file_id, file_name) in file_names.iter().enumerate() {
            //free_clusters_before_create.push(fs.num_free_clusters());
            let inode = create_file(root.clone(), file_name);

            if fs.num_free_clusters() > file_id as u32 {
                let _ = inode.write_bytes_at(file_id * cluster_size, &[0, 1, 2, 3, 4]);
            }
        }

        let mut reverse_names = file_names.clone();
        reverse_names.reverse();
        for (file_id, file_name) in reverse_names.iter().enumerate() {
            let id = file_num as usize - 1 - file_id;
            let unlink_result = root.unlink(file_name);
            assert!(unlink_result.is_ok(), "Fail to unlink file {:?}", id);

            // assert!(
            //     fs.num_free_clusters() == free_clusters_before_create[id],
            //     "Space is still occupied after unlinking"
            // );

            let mut sub_inodes: Vec<String> = Vec::new();

            let read_result = root.readdir_at(0, &mut sub_inodes);
            assert!(
                read_result.is_ok(),
                "Fail to readdir after unlink {:?}: {:?}",
                id,
                read_result.unwrap_err()
            );

            assert!(read_result.unwrap() == id + 2);
            assert!(sub_inodes.len() == id + 2);

            sub_inodes.remove(0);
            sub_inodes.remove(0);
            sub_inodes.sort();

            for i in 0..sub_inodes.len() {
                assert!(
                    sub_inodes[i].cmp(&file_names[i]).is_eq(),
                    "File name mismatch at {:?}: read {:?} expect {:?}",
                    i,
                    sub_inodes[i],
                    file_names[i]
                );
            }
        }
    }

    #[ktest]
    fn rmdir() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let folder_name = "sub";
        create_folder(root.clone(), folder_name);
        let rmdir_result = root.rmdir(folder_name);
        assert!(
            rmdir_result.is_ok(),
            "Fail to rmdir: {:?}",
            rmdir_result.unwrap_err()
        );

        let mut sub_dirs: Vec<String> = Vec::new();
        let _ = root.readdir_at(0, &mut sub_dirs);
        assert!(sub_dirs.len() == 2);

        // Followings are some invalid unlink call. These should return with an error.
        let rmdir_fail_result1 = root.rmdir(".");
        assert!(
            rmdir_fail_result1.is_err(),
            "Fs deal with rmdir(.) incorrectly"
        );

        let rmdir_fail_result2 = root.rmdir("..");
        assert!(
            rmdir_fail_result2.is_err(),
            "Fs deal with rmdir(..) incorrectly"
        );

        let file_name = "a.txt";
        create_file(root.clone(), file_name);
        let rmdir_to_a_file = root.rmdir(file_name);
        assert!(
            rmdir_to_a_file.is_err(),
            "Fs deal with rmdir to a file incorrectly"
        );

        let parent_name = "parent";
        let child_name = "child.txt";
        let parent_inode = create_folder(root.clone(), parent_name);
        create_file(parent_inode.clone(), child_name);
        let rmdir_no_empty_dir = root.rmdir(parent_name);
        assert!(
            rmdir_no_empty_dir.is_err(),
            "Fs deal with rmdir to a no empty directory incorrectly"
        );
        // however, after we remove child file, parent directory is removable.
        let _ = parent_inode.unlink(child_name);
        let rmdir_empty_dir = root.rmdir(parent_name);
        assert!(rmdir_empty_dir.is_ok(), "Fail to remove an empty directory");

        let _parent_inode_again = create_folder(root.clone(), parent_name);
        create_file(parent_inode.clone(), child_name);
        let lookup_result = parent_inode.lookup(child_name);
        assert!(
            lookup_result.is_ok(),
            "Fs deal with second create incorrectly, may need check pagecache"
        );

        // test remove a long name directory
        let long_dir_name = "x".repeat(MAX_NAME_LENGTH);
        create_folder(root.clone(), &long_dir_name);
        let rmdir_long_name_dir = root.rmdir(&long_dir_name);
        assert!(
            rmdir_long_name_dir.is_ok(),
            "Fail to remove a long name directory"
        );
    }

    #[ktest]
    fn rename_file() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let file_name = "HI.TXT";
        let a_inode = create_file(root.clone(), file_name);

        const BUF_SIZE: usize = 7 * PAGE_SIZE + 11;
        let mut buf = vec![0u8; BUF_SIZE];
        for (i, num) in buf.iter_mut().enumerate() {
            //Use a prime number to make each sector different.
            *num = (i % 107) as u8;
        }
        let _ = a_inode.write_bytes_at(0, &buf);

        let new_name = "HELLO.TXT";
        let rename_result = root.rename(file_name, &root.clone(), new_name);
        assert!(
            rename_result.is_ok(),
            "Failed to rename: {:?}",
            rename_result.unwrap_err()
        );

        // test list after rename
        let mut sub_dirs: Vec<String> = Vec::new();
        let _ = root.readdir_at(0, &mut sub_dirs);
        assert!(sub_dirs.len() == 3 && sub_dirs[2].eq(new_name));

        // test read after rename
        let a_inode_new = root.lookup(new_name).unwrap();
        let mut read = vec![0u8; BUF_SIZE];
        let read_after_rename = a_inode_new.read_bytes_at(0, &mut read);
        assert!(
            read_after_rename.is_ok() && read_after_rename.unwrap() == BUF_SIZE,
            "Fail to read after rename: {:?}",
            read_after_rename.unwrap_err()
        );
        assert!(buf.eq(&read), "File mismatch after rename");

        // test write after rename
        const NEW_BUF_SIZE: usize = 9 * PAGE_SIZE + 23;
        let new_buf = vec![7u8; NEW_BUF_SIZE];
        let new_write_after_rename = a_inode_new.write_bytes_at(0, &new_buf);
        assert!(
            new_write_after_rename.is_ok() && new_write_after_rename.unwrap() == NEW_BUF_SIZE,
            "Fail to write file after rename: {:?}",
            new_write_after_rename.unwrap_err()
        );

        let mut new_read = vec![0u8; NEW_BUF_SIZE];
        let _ = a_inode_new.read_bytes_at(0, &mut new_read);
        assert!(
            new_buf.eq(&new_read),
            "New read and new write mismatch after rename"
        );

        // test rename between different directories
        let sub_folder_name = "TEST";
        let sub_folder = create_folder(root.clone(), sub_folder_name);
        let sub_file_name = "A.TXT";
        create_file(sub_folder.clone(), sub_file_name);
        let rename_result = sub_folder.rename(sub_file_name, &root.clone(), sub_file_name);
        assert!(
            rename_result.is_ok(),
            "Fs failed to rename file between different directories: {:?}",
            rename_result.unwrap_err()
        );

        sub_dirs.clear();

        let _ = root.readdir_at(0, &mut sub_dirs);
        sub_dirs.remove(0);
        sub_dirs.remove(0);

        sub_dirs.sort();

        assert!(
            sub_dirs.len() == 3
                && sub_dirs[0].eq(sub_file_name)
                && sub_dirs[1].eq(new_name)
                && sub_dirs[2].eq(sub_folder_name)
        );

        // test rename file when the new_name is exist
        let rename_file_to_itself = root.rename(new_name, &root.clone(), new_name);
        assert!(rename_file_to_itself.is_ok(), "Fail to rename to itself");

        let rename_file_to_an_exist_folder = root.rename(new_name, &root.clone(), sub_folder_name);
        assert!(
            rename_file_to_an_exist_folder.is_err(),
            "Fs deal with rename a file to an exist directory incorrectly"
        );

        let rename_file_to_an_exist_file = root.rename(new_name, &root.clone(), sub_file_name);
        assert!(
            rename_file_to_an_exist_file.is_ok(),
            "Fail to rename a file to another exist file",
        );

        sub_dirs.clear();
        let _ = root.readdir_at(0, &mut sub_dirs);
        sub_dirs.remove(0);
        sub_dirs.remove(0);
        sub_dirs.sort();

        assert!(
            sub_dirs.len() == 2 && sub_dirs[0].eq(sub_file_name) && sub_dirs[1].eq(sub_folder_name)
        );
    }

    #[ktest]
    fn rename_dir() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let old_folder_name = "OLD_FOLDER";
        let old_folder = create_folder(root.clone(), old_folder_name);
        let child_file_name = "A.TXT";
        create_file(old_folder.clone(), child_file_name);

        // Test rename a folder, the sub-directories should remain.
        let new_folder_name = "NEW_FOLDER";
        let rename_result = root.rename(old_folder_name, &root.clone(), new_folder_name);

        assert!(
            rename_result.is_ok(),
            "Fs failed to rename a folder: {:?}",
            rename_result.unwrap_err()
        );

        let mut sub_dirs: Vec<String> = Vec::new();
        let _ = root.readdir_at(0, &mut sub_dirs);
        assert!(sub_dirs.len() == 3 && sub_dirs[2].eq(new_folder_name));

        let new_folder = root.lookup(new_folder_name).unwrap();

        sub_dirs.clear();
        let _ = new_folder.readdir_at(0, &mut sub_dirs);
        assert!(sub_dirs.len() == 3 && sub_dirs[2].eq(child_file_name));

        // Test rename directory when the new_name is exist.
        let exist_folder_name = "EXIST_FOLDER";
        let exist_folder = create_folder(root.clone(), exist_folder_name);
        create_file(exist_folder.clone(), child_file_name);

        let exist_file_name = "EXIST_FILE.TXT";
        create_file(root.clone(), exist_file_name);

        let rename_dir_to_an_exist_file =
            root.rename(new_folder_name, &root.clone(), exist_file_name);

        assert!(rename_dir_to_an_exist_file.is_err());

        let rename_dir_to_an_exist_no_empty_folder =
            root.rename(new_folder_name, &root.clone(), exist_folder_name);
        assert!(rename_dir_to_an_exist_no_empty_folder.is_err());

        let _ = exist_folder.unlink(child_file_name);

        let rename_dir_to_an_exist_empty_folder =
            root.rename(new_folder_name, &root.clone(), exist_folder_name);
        assert!(rename_dir_to_an_exist_empty_folder.is_ok());
    }

    #[ktest]
    fn write_and_read_file_direct() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let file = create_file(root.clone(), "test");

        // const BUF_SIZE: usize = PAGE_SIZE * 7 + 3 * SECTOR_SIZE;
        const BUF_SIZE: usize = PAGE_SIZE * 7;

        let mut buf = vec![0u8; BUF_SIZE];
        for (i, num) in buf.iter_mut().enumerate() {
            //Use a prime number to make each sector different.
            *num = (i % 107) as u8;
        }

        let write_result = file.write_bytes_direct_at(0, &buf);
        assert!(
            write_result.is_ok(),
            "Fs failed to write direct: {:?}",
            write_result.unwrap_err()
        );

        let mut read = vec![0u8; BUF_SIZE];
        let read_result = file.read_bytes_direct_at(0, &mut read);
        assert!(
            read_result.is_ok(),
            "Fs failed to read direct: {:?}",
            read_result.unwrap_err()
        );

        assert!(buf.eq(&read), "File mismatch. Data read result:{:?}", read);
    }

    #[ktest]
    fn write_and_read_file() {
        let fs = load_exfat();
        let root = fs.root_inode() as Arc<dyn Inode>;
        let file = create_file(root.clone(), "test");

        const BUF_SIZE: usize = PAGE_SIZE * 11 + 2023;

        let mut buf = vec![0u8; BUF_SIZE];
        for (i, num) in buf.iter_mut().enumerate() {
            //Use a prime number to make each sector different.
            *num = (i % 107) as u8;
        }

        let write_result = file.write_bytes_at(0, &buf);
        assert!(
            write_result.is_ok(),
            "Fs failed to write: {:?}",
            write_result.unwrap_err()
        );

        let mut read = vec![0u8; BUF_SIZE];
        let read_result = file.read_bytes_at(0, &mut read);
        assert!(
            read_result.is_ok(),
            "Fs failed to read: {:?}",
            read_result.unwrap_err()
        );

        assert!(buf.eq(&read), "File mismatch. Data read result:{:?}", read);
    }

    #[ktest]
    fn interleaved_write() {
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
        let write_len = BUF_SIZE.div_ceil(steps);
        for i in 0..steps {
            let start = i * write_len;
            let end = BUF_SIZE.min(start + write_len);
            a.write_bytes_at(start, &buf_a[start..end]).unwrap();
            b.write_bytes_at(start, &buf_b[start..end]).unwrap();
        }

        let mut read = vec![0u8; BUF_SIZE];
        a.read_bytes_at(0, &mut read).unwrap();
        assert!(
            buf_a.eq(&read),
            "File a mismatch. Data read result:{:?}",
            read
        );

        b.read_bytes_at(0, &mut read).unwrap();
        assert!(
            buf_b.eq(&read),
            "File b mismatch. Data read result:{:?}",
            read
        );
    }

    #[ktest]
    fn bitmap_modify_bit() {
        let fs = load_exfat();
        let bitmap_binding = fs.bitmap();
        let mut bitmap = bitmap_binding.lock();
        let total_bits_len = 200;
        let initial_free_clusters = bitmap.num_free_clusters();

        let range_result =
            bitmap.find_next_unused_cluster_range(EXFAT_RESERVED_CLUSTERS, total_bits_len);
        assert!(
            range_result.is_ok(),
            "Fail to get a free range with {:?} clusters",
            total_bits_len
        );

        let range_start_cluster = range_result.unwrap().start;
        let p = 107;
        for i in 0..total_bits_len {
            let relative_idx = (i * p) % total_bits_len;
            let idx = range_start_cluster + relative_idx;
            let res1 = bitmap.is_cluster_unused(idx);
            assert!(
                res1.is_ok() && res1.unwrap(),
                "Cluster idx {:?} is set before set",
                relative_idx
            );

            let res2 = bitmap.set_used(idx, true);
            assert!(
                res2.is_ok() && bitmap.num_free_clusters() == initial_free_clusters - 1,
                "Set cluster idx {:?} failed",
                relative_idx
            );

            let res3 = bitmap.is_cluster_unused(idx);
            assert!(
                res3.is_ok() && !res3.unwrap(),
                "Cluster idx {:?} is unset after set",
                relative_idx
            );

            let res4 = bitmap.set_unused(idx, true);
            assert!(
                res4.is_ok() && bitmap.num_free_clusters() == initial_free_clusters,
                "Clear cluster idx {:?} failed",
                relative_idx
            );

            let res5 = bitmap.is_cluster_unused(idx);
            assert!(
                res5.is_ok() && res5.unwrap(),
                "Cluster idx {:?} is still set after clear",
                relative_idx
            );
        }
    }

    #[ktest]
    fn bitmap_modify_chunk() {
        let fs = load_exfat();
        let bitmap_binding = fs.bitmap();
        let mut bitmap = bitmap_binding.lock();
        let total_bits_len = 1000;
        let initial_free_clusters = bitmap.num_free_clusters();

        let range_result =
            bitmap.find_next_unused_cluster_range(EXFAT_RESERVED_CLUSTERS, total_bits_len);
        assert!(
            range_result.is_ok(),
            "Fail to get a free range with {:?} clusters",
            total_bits_len
        );

        let range_start_idx = range_result.unwrap().start;
        let mut chunk_size = 1;
        let mut start_idx: u32 = range_start_idx;
        let mut end_idx = range_start_idx + 1;
        while end_idx <= range_start_idx + total_bits_len {
            let res1 = bitmap.set_range_used(start_idx..end_idx, true);
            assert!(
                res1.is_ok() && bitmap.num_free_clusters() == initial_free_clusters - chunk_size,
                "Set cluster chunk [{:?}, {:?}) failed",
                start_idx,
                end_idx
            );

            for idx in start_idx..end_idx {
                let res = bitmap.is_cluster_unused(idx);
                assert!(
                    res.is_ok() && !res.unwrap(),
                    "Cluster {:?} in chunk [{:?}, {:?}) is unset",
                    idx,
                    start_idx,
                    end_idx
                );
            }

            let res2 = bitmap.set_range_unused(start_idx..end_idx, true);
            assert!(
                res2.is_ok() && bitmap.num_free_clusters() == initial_free_clusters,
                "Clear cluster chunk [{:?}, {:?}) failed",
                start_idx,
                end_idx
            );

            let res3 = bitmap.is_cluster_range_unused(start_idx..end_idx);
            assert!(
                res3.is_ok() && res3.unwrap(),
                "Some bit in cluster chunk [{:?}, {:?}) is still set after clear",
                start_idx,
                end_idx
            );

            chunk_size += 1;
            start_idx = end_idx;
            end_idx = start_idx + chunk_size;
        }
    }

    #[ktest]
    fn bitmap_find() {
        let fs = load_exfat();
        let bitmap_binding = fs.bitmap();
        let mut bitmap = bitmap_binding.lock();
        let total_bits_len = 1000;

        let range_result =
            bitmap.find_next_unused_cluster_range(EXFAT_RESERVED_CLUSTERS, total_bits_len);
        assert!(
            range_result.is_ok(),
            "Fail to get a free range with {:?} clusters",
            total_bits_len
        );

        let range_start_idx = range_result.unwrap().start;
        let mut chunk_size = 1;
        let mut start_idx;
        let mut end_idx = range_start_idx + 1;
        // 010010001000010000010000001...
        // chunk_size = k, relative_start_idx =(k-1)*(k+2)/2
        while end_idx <= range_start_idx + total_bits_len {
            let _ = bitmap.set_used(end_idx, true);
            chunk_size += 1;
            start_idx = end_idx + 1;
            end_idx = start_idx + chunk_size;
        }

        for k in 1..chunk_size {
            let start_idx_k = bitmap.find_next_unused_cluster_range(range_start_idx, k);
            assert!(
                start_idx_k.is_ok()
                    && start_idx_k.clone().unwrap().start
                        == (k - 1) * (k + 2) / 2 + range_start_idx
                    && start_idx_k.unwrap().end == (k * k + 3 * k - 2) / 2 + range_start_idx,
                "Fail to find chunk size {:?}",
                k
            );
        }
    }

    #[ktest]
    fn resize_single_file() {
        let fs = load_exfat();
        let root = fs.root_inode();
        let f = create_file(root.clone(), "xxx");
        let cluster_size = fs.cluster_size();
        let initial_free_clusters = fs.num_free_clusters();

        let max_clusters = 100.min(initial_free_clusters);
        let mut alloc_clusters = 0;
        while alloc_clusters < max_clusters {
            alloc_clusters += 1;
            info!("alloc_clusters = {:?}", alloc_clusters);
            let resize_result = f.resize(alloc_clusters as usize * cluster_size);
            assert!(
                resize_result.is_ok()
                    && fs.num_free_clusters() == initial_free_clusters - alloc_clusters,
                "Fail to linearly expand file to {:?} clusters",
                alloc_clusters
            );
        }
        // here alloc_clusters == max_clusters

        while alloc_clusters > 0 {
            alloc_clusters -= 1;
            let resize_result = f.resize(alloc_clusters as usize * cluster_size);
            assert!(
                resize_result.is_ok()
                    && fs.num_free_clusters() == initial_free_clusters - alloc_clusters,
                "Fail to linearly shrink file to {:?} clusters",
                alloc_clusters
            );
        }

        alloc_clusters = 1;
        let mut old_alloc_clusters = 0;
        let mut step = 1;
        while alloc_clusters <= max_clusters {
            let resize_result = f.resize(alloc_clusters as usize * cluster_size);
            assert!(
                resize_result.is_ok()
                    && fs.num_free_clusters() == initial_free_clusters - alloc_clusters,
                "Fail to expand file from {:?} clusters to {:?} clusters",
                old_alloc_clusters,
                alloc_clusters
            );
            old_alloc_clusters = alloc_clusters;
            step += 1;
            alloc_clusters += step;
        }

        while alloc_clusters > 0 {
            alloc_clusters -= step;
            step -= 1;
            let resize_result = f.resize(alloc_clusters as usize * cluster_size);
            assert!(
                resize_result.is_ok()
                    && fs.num_free_clusters() == initial_free_clusters - alloc_clusters,
                "Fail to shrink file from {:?} clusters to {:?} clusters",
                old_alloc_clusters,
                alloc_clusters
            );
            old_alloc_clusters = alloc_clusters;
        }
        assert!(alloc_clusters == 0);

        // Try to allocate a file larger than remaining spaces. This will fail without changing the remaining space.
        let resize_too_large = f.resize(initial_free_clusters as usize * cluster_size + 1);
        assert!(
            resize_too_large.is_err() && fs.num_free_clusters() == initial_free_clusters,
            "Fail to deal with a memory overflow allocation"
        );

        // Try to allocate a file of exactly the same size as the remaining spaces. This will succeed.
        let resize_exact = f.resize(initial_free_clusters as usize * cluster_size);
        assert!(
            resize_exact.is_ok() && fs.num_free_clusters() == 0,
            "Fail to deal with a exact allocation"
        );

        // Free the file just allocated. This will also succeed.
        let free_all = f.resize(0);
        assert!(
            free_all.is_ok() && fs.num_free_clusters() == initial_free_clusters,
            "Fail to free a large chunk"
        );
    }

    #[ktest]
    fn resize_multiple_files() {
        let fs = load_exfat();
        let cluster_size = fs.cluster_size();
        let root = fs.root_inode();
        let file_num: u32 = 45;
        let mut file_names: Vec<String> = (0..file_num).map(|x| x.to_string()).collect();
        file_names.sort();
        let mut file_inodes: Vec<Arc<dyn Inode>> = Vec::new();
        for file_name in file_names.iter() {
            let inode = create_file(root.clone(), file_name);
            file_inodes.push(inode);
        }

        let initial_free_clusters = fs.num_free_clusters();
        let max_clusters = 1000.min(initial_free_clusters);
        let mut step = 1;
        let mut cur_clusters_per_file = 0;
        while file_num * (cur_clusters_per_file + step) <= max_clusters {
            for (file_id, inode) in file_inodes.iter().enumerate() {
                let resize_result =
                    inode.resize((cur_clusters_per_file + step) as usize * cluster_size);
                assert!(
                    resize_result.is_ok()
                        && fs.num_free_clusters()
                            == initial_free_clusters
                                - cur_clusters_per_file * file_num
                                - (file_id as u32 + 1) * step,
                    "Fail to resize file {:?} from {:?} to {:?}",
                    file_id,
                    cur_clusters_per_file,
                    cur_clusters_per_file + step
                );
            }
            cur_clusters_per_file += step;
            step += 1;
        }
    }

    #[ktest]
    fn resize_and_write() {
        let fs = load_exfat();
        let root = fs.root_inode();
        let inode = root
            .create("xxx", InodeType::File, InodeMode::all())
            .unwrap();
        const MAX_PAGE_PER_FILE: usize = 20;
        let mut rng = SmallRng::seed_from_u64(0);

        let mut buf: Vec<u8> = Vec::new();
        let mut pg_num = 1;
        while pg_num <= MAX_PAGE_PER_FILE {
            let size = pg_num * PAGE_SIZE;
            let _ = inode.resize(size);

            buf.resize(size, 0);
            rng.fill_bytes(&mut buf);
            let write_result = inode.write_bytes_at(0, &buf);
            assert!(
                write_result.is_ok(),
                "Fail to write after resize expand from {:?}pgs to {:?}pgs: {:?}",
                pg_num - 1,
                pg_num,
                write_result.unwrap_err()
            );

            pg_num += 1;
        }

        pg_num = MAX_PAGE_PER_FILE;

        while pg_num > 0 {
            let size = (pg_num - 1) * PAGE_SIZE;
            let _ = inode.resize(size);

            buf.resize(size, 0);
            rng.fill_bytes(&mut buf);
            let write_result = inode.write_bytes_at(0, &buf);
            assert!(
                write_result.is_ok(),
                "Fail to write after resize shrink from {:?}pgs to {:?}pgs: {:?}",
                pg_num,
                pg_num - 1,
                write_result.unwrap_err()
            );

            pg_num -= 1;
        }
    }

    #[ktest]
    fn random_op_sequence() {
        let fs = load_exfat();
        let root = fs.root_inode();
        let mut fs_in_mem = new_fs_in_memory(root);
        let mut rng = SmallRng::seed_from_u64(0);

        let max_ops: u32 = 500;

        for idx in 0..max_ops {
            let (file_or_dir, op) = generate_random_operation(&mut fs_in_mem, idx, &mut rng);
            file_or_dir.execute_and_test(op, &mut rng);
        }
    }
}
