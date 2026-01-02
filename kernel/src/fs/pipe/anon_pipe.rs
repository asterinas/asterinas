// SPDX-License-Identifier: MPL-2.0

use alloc::format;
use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use crate::{
    fs::{
        file_handle::FileLike,
        inode_handle::{FileIo, InodeHandle},
        notify::FsEventPublisher,
        path::Path,
        pipe::Pipe,
        pseudofs::{PseudoInode, pipefs_mount, pipefs_singleton},
        utils::{
            AccessMode, FileSystem, Inode, InodeIo, InodeMode, InodeType, Metadata, StatusFlags,
            mkmod,
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

/// Creates a pair of connected pipe file handles with the default capacity.
pub fn new_file_pair() -> Result<(Arc<InodeHandle>, Arc<InodeHandle>)> {
    let pipe_inode = Arc::new(AnonPipeInode::new());

    let path = Path::new_pseudo(pipefs_mount().clone(), pipe_inode, |inode| {
        format!("pipe:[{}]", inode.ino())
    });

    let reader = InodeHandle::new_unchecked_access(
        path.clone(),
        AccessMode::O_RDONLY,
        StatusFlags::empty(),
    )?;
    let writer =
        InodeHandle::new_unchecked_access(path, AccessMode::O_WRONLY, StatusFlags::empty())?;

    Ok((Arc::new(reader), Arc::new(writer)))
}

/// An anonymous pipe inode.
pub struct AnonPipeInode {
    /// The underlying pipe backend.
    pipe: Pipe,
    pseudo_inode: PseudoInode,
}

impl AnonPipeInode {
    fn new() -> Self {
        let pipe = Pipe::new();

        let pseudo_inode = pipefs_singleton().alloc_inode(
            InodeType::NamedPipe,
            mkmod!(u+rw),
            Uid::new_root(),
            Gid::new_root(),
        );

        Self { pipe, pseudo_inode }
    }
}

#[inherit_methods(from = "self.pseudo_inode")]
impl InodeIo for AnonPipeInode {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status: StatusFlags,
    ) -> Result<usize>;
    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status: StatusFlags,
    ) -> Result<usize>;
}

#[inherit_methods(from = "self.pseudo_inode")]
impl Inode for AnonPipeInode {
    fn size(&self) -> usize;
    fn resize(&self, _new_size: usize) -> Result<()>;
    fn metadata(&self) -> Metadata;
    fn fs_event_publisher(&self) -> &FsEventPublisher;
    fn ino(&self) -> u64;
    fn type_(&self) -> InodeType;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        Some(self.pipe.open_anon(access_mode, status_flags))
    }
}

#[cfg(ktest)]
mod test {
    use alloc::sync::Arc;
    use core::sync::atomic::{self, AtomicBool};

    use ostd::prelude::*;

    use super::*;
    use crate::thread::{Thread, kernel_thread::ThreadOptions};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ordering {
        WriteThenRead,
        ReadThenWrite,
    }

    fn test_blocking<W, R>(write: W, read: R, ordering: Ordering)
    where
        W: FnOnce(Arc<InodeHandle>) + Send + 'static,
        R: FnOnce(Arc<InodeHandle>) + Send + 'static,
    {
        let (reader, writer) = new_file_pair().unwrap();

        let signal_writer = Arc::new(AtomicBool::new(false));
        let signal_reader = signal_writer.clone();

        let writer = ThreadOptions::new(move || {
            if ordering == Ordering::ReadThenWrite {
                while !signal_writer.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_writer.store(true, atomic::Ordering::Relaxed);
            }

            write(writer);
        })
        .spawn();

        let reader = ThreadOptions::new(move || {
            if ordering == Ordering::WriteThenRead {
                while !signal_reader.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_reader.store(true, atomic::Ordering::Relaxed);
            }

            read(reader);
        })
        .spawn();

        writer.join();
        reader.join();
    }

    #[ktest]
    fn test_read_empty() {
        test_blocking(
            |writer| {
                assert_eq!(writer.write(&mut reader_from(&[1])).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 1];
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 1);
                assert_eq!(&buf, &[1]);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn test_write_full() {
        test_blocking(
            |writer| {
                assert_eq!(writer.write(&mut reader_from(&[1, 2, 3])).unwrap(), 2);
                assert_eq!(writer.write(&mut reader_from(&[2])).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 1);
                assert_eq!(&buf[..1], &[2]);
            },
            Ordering::WriteThenRead,
        );
    }

    #[ktest]
    fn test_read_closed() {
        test_blocking(
            drop,
            |reader| {
                let mut buf = [0; 1];
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 0);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn test_write_closed() {
        test_blocking(
            |writer| {
                assert_eq!(writer.write(&mut reader_from(&[1, 2, 3])).unwrap(), 2);
                assert_eq!(
                    writer.write(&mut reader_from(&[2])).unwrap_err().error(),
                    Errno::EPIPE
                );
            },
            drop,
            Ordering::WriteThenRead,
        );
    }

    #[ktest]
    fn test_write_atomicity() {
        test_blocking(
            |writer| {
                assert_eq!(writer.write(&mut reader_from(&[1])).unwrap(), 1);
                assert_eq!(writer.write(&mut reader_from(&[1, 2])).unwrap(), 2);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 1);
                assert_eq!(&buf[..1], &[1]);
                assert_eq!(reader.read(&mut writer_from(&mut buf)).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
            },
            Ordering::WriteThenRead,
        );
    }

    fn reader_from(buf: &[u8]) -> VmReader<'_> {
        VmReader::from(buf).to_fallible()
    }

    fn writer_from(buf: &mut [u8]) -> VmWriter<'_> {
        VmWriter::from(buf).to_fallible()
    }
}
