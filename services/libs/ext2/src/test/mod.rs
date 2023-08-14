use crate::fs::{Ext2, EXT2_ROOT_INO};
use crate::inode::{FilePerm, FileType, FAST_SYMLINK_MAX_LEN};
use crate::prelude::*;
use block_io::bio::BioType;

use std::fs::File;
use std::os::unix::prelude::FileExt;
use std::sync::Mutex;

use self::vnode::Vnode;

mod vnode;

#[derive(Debug)]
struct FileBlock(Mutex<File>);

impl BlockDevice for FileBlock {
    fn submit_bio(&self, bio: &mut Bio) -> mem_storage::Result<usize> {
        let start_idx = bio.idx();
        let mut num_processed = 0;
        match bio.bio_type() {
            BioType::Read => {
                for bio_buf_des in bio.bio_bufs_mut().iter_mut().skip(start_idx) {
                    let offset = bio_buf_des.bid().to_offset() + bio_buf_des.offset();
                    let bio_buf = bio_buf_des.buf_mut();
                    match self
                        .0
                        .lock()
                        .unwrap()
                        .read_exact_at(bio_buf.as_mut_slice(), offset as _)
                    {
                        Ok(_) => num_processed += 1,
                        Err(_) if num_processed == 0 => {
                            return Err(mem_storage::Error::IoError);
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
            }
            BioType::Write => {
                for bio_buf_des in bio.bio_bufs().iter().skip(start_idx) {
                    let offset = bio_buf_des.bid().to_offset() + bio_buf_des.offset();
                    let bio_buf = bio_buf_des.buf();
                    match self
                        .0
                        .lock()
                        .unwrap()
                        .write_all_at(bio_buf.as_slice(), offset as _)
                    {
                        Ok(_) => num_processed += 1,
                        Err(_) if num_processed == 0 => {
                            return Err(mem_storage::Error::IoError);
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
            }
        }
        bio.set_idx(start_idx + num_processed);
        Ok(num_processed)
    }

    fn total_blocks(&self) -> BlockId {
        BlockId::from_offset(self.0.lock().unwrap().metadata().unwrap().len() as _)
    }
}

lazy_static! {
    static ref EXT2FS: Arc<Ext2> = {
        let file = Box::new(FileBlock(Mutex::new(
            File::options()
                .read(true)
                .write(true)
                .open("/root/ext2.image")
                .unwrap(),
        )));
        let ext2 = Ext2::open(file).unwrap();
        info!("ext2: {:?}", ext2);
        ext2
    };
}

// cargo test -- --nocapture

#[test]
fn test_root_inode() {
    let root_inode = EXT2FS.root_inode().unwrap();
    assert!(root_inode.ino() == EXT2_ROOT_INO);

    let root_inode = Vnode::new(EXT2FS.root_inode().unwrap()).unwrap();
    info!("root inode: {:?}", root_inode);
}

#[test]
fn test_dir() {
    let root_inode = Vnode::new(EXT2FS.root_inode().unwrap()).unwrap();
    let _ = root_inode
        .create("dir", FileType::Dir, FilePerm::from_bits_truncate(0o755))
        .unwrap();
    let dir_inode = root_inode.lookup("dir").unwrap();
    info!("/dir inode: {:?}", dir_inode);
    let _ = dir_inode
        .create("file", FileType::File, FilePerm::from_bits_truncate(0o666))
        .unwrap();
    let file_inode = dir_inode.lookup("file").unwrap();
    info!("/dir/file inode: {:?}", file_inode);
}

#[test]
fn test_file() {
    let root_inode = Vnode::new(EXT2FS.root_inode().unwrap()).unwrap();

    let file_inode = root_inode
        .create("file", FileType::File, FilePerm::from_bits_truncate(0o666))
        .unwrap();
    info!("/file inode: {:?}", file_inode);

    const HELLO_WORLD_STR: &str = "hello,world";
    file_inode.write_at(0, HELLO_WORLD_STR.as_bytes()).unwrap();
    drop(file_inode);

    let file_inode = root_inode.lookup("file").unwrap();
    info!("/file inode: {:?}", file_inode);
    let mut read_vec = vec![0u8; HELLO_WORLD_STR.len()];
    file_inode.read_at(0, &mut read_vec).unwrap();
    assert_eq!(HELLO_WORLD_STR.as_bytes(), read_vec.as_slice());
}

#[test]
fn test_symlink() {
    let root_inode = Vnode::new(EXT2FS.root_inode().unwrap()).unwrap();

    const TARGET_STR: &str = "/file";
    let sym_inode = root_inode
        .create(
            "sym",
            FileType::Symlink,
            FilePerm::from_bits_truncate(0o777),
        )
        .unwrap();
    sym_inode.write_link(TARGET_STR).unwrap();
    info!("/sym inode: {:?}", sym_inode);
    drop(sym_inode);

    let sym_inode = root_inode.lookup("sym").unwrap();
    let link_content = sym_inode.read_link().unwrap();
    assert_eq!(TARGET_STR, link_content.as_str());

    const LONG_STR: &str = "0123456789012345678901234567890123456789012345678901234567890123456789";
    assert!(LONG_STR.len() > FAST_SYMLINK_MAX_LEN);
    sym_inode.write_link(LONG_STR).unwrap();
    info!("new /sym inode: {:?}", sym_inode);
    let link_content = sym_inode.read_link().unwrap();
    assert_eq!(LONG_STR, link_content.as_str());
}
