// SPDX-License-Identifier: MPL-2.0

use super::*;

impl FileSystemDevice {
    pub(crate) fn fuse_init(&self) -> Result<(), VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<InitIn>()) as u32,
            FuseOpcode::Init.into(),
            unique,
            0,
        );
        let init_in = InitIn::new(FUSE_KERNEL_VERSION, FUSE_KERNEL_MINOR_VERSION, 0, 0, 0);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, init_in, size_of::<InitOut>())?;

        let selector = QueueSelector::Request(0);
        let request = self.submit_request(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.wait_for_request_early(selector, &request)?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let init_out: InitOut = out_payload_slice.read_val(0).unwrap();

        info!(
            "{} FUSE session started: protocol {}.{} -> {}.{}, max_write={}, flags=0x{:x}",
            DEVICE_NAME,
            FUSE_KERNEL_VERSION,
            FUSE_KERNEL_MINOR_VERSION,
            init_out.major,
            init_out.minor,
            init_out.max_write,
            init_out.flags,
        );

        Ok(())
    }

    pub fn fuse_lookup(
        &self,
        parent_nodeid: u64,
        name: &str,
    ) -> Result<EntryOut, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + name.len() + 1) as u32,
            FuseOpcode::Lookup.into(),
            unique,
            parent_nodeid,
        );

        let in_header_slice = self.prepare_in_header_buf(in_header)?;
        let in_name_slice = self.prepare_in_name_buf(name)?;

        let out_header_slice = self.prepare_out_header_buf()?;
        let out_payload_slice = self.prepare_out_payload_buf(size_of::<EntryOut>())?;

        let selector = self.select_request_queue(parent_nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_name_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let entry_out: EntryOut = out_payload_slice.read_val(0).unwrap();

        Ok(entry_out)
    }

    pub fn fuse_mkdir(
        &self,
        parent_nodeid: u64,
        name: &str,
        mode: u32,
    ) -> Result<EntryOut, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<MkdirIn>() + name.len() + 1) as u32,
            FuseOpcode::Mkdir.into(),
            unique,
            parent_nodeid,
        );
        let mkdir_in = MkdirIn::new(mode);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, mkdir_in, size_of::<EntryOut>())?;

        let in_name_slice = self.prepare_in_name_buf(name)?;

        let selector = self.select_request_queue(parent_nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice, &in_name_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let entry_out: EntryOut = out_payload_slice.read_val(0).unwrap();

        Ok(entry_out)
    }

    pub fn fuse_mknod(
        &self,
        parent_nodeid: u64,
        name: &str,
        mode: u32,
        rdev: u32,
    ) -> Result<EntryOut, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<MknodIn>() + name.len() + 1) as u32,
            FuseOpcode::Mknod.into(),
            unique,
            parent_nodeid,
        );
        let mknod_in = MknodIn::new(mode, rdev);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, mknod_in, size_of::<EntryOut>())?;

        let in_name_slice = self.prepare_in_name_buf(name)?;

        let selector = self.select_request_queue(parent_nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice, &in_name_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let entry_out: EntryOut = out_payload_slice.read_val(0).unwrap();

        Ok(entry_out)
    }

    pub fn fuse_unlink(&self, parent_nodeid: u64, name: &str) -> Result<(), VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + name.len() + 1) as u32,
            FuseOpcode::Unlink.into(),
            unique,
            parent_nodeid,
        );

        let in_header_slice = self.prepare_in_header_buf(in_header)?;
        let in_name_slice = self.prepare_in_name_buf(name)?;

        let out_header_slice = self.prepare_out_header_buf()?;

        let selector = self.select_request_queue(parent_nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_name_slice],
            &[&out_header_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        Ok(())
    }

    pub fn fuse_rmdir(&self, parent_nodeid: u64, name: &str) -> Result<(), VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + name.len() + 1) as u32,
            FuseOpcode::Rmdir.into(),
            unique,
            parent_nodeid,
        );

        let in_header_slice = self.prepare_in_header_buf(in_header)?;
        let in_name_slice = self.prepare_in_name_buf(name)?;

        let out_header_slice = self.prepare_out_header_buf()?;
        let selector = self.select_request_queue(parent_nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_name_slice],
            &[&out_header_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        Ok(())
    }

    pub fn fuse_create(
        &self,
        parent_nodeid: u64,
        name: &str,
        mode: u32,
    ) -> Result<(EntryOut, OpenOut), VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<CreateIn>() + name.len() + 1) as u32,
            FuseOpcode::Create.into(),
            unique,
            parent_nodeid,
        );

        let create_in = CreateIn::new(O_RDWR, mode);

        let out_payload_size = size_of::<EntryOut>() + size_of::<OpenOut>();
        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, create_in, out_payload_size)?;

        let in_name_slice = self.prepare_in_name_buf(name)?;

        let selector = self.select_request_queue(parent_nodeid);

        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice, &in_name_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let entry_out: EntryOut = out_payload_slice.read_val(0).unwrap();
        let open_out: OpenOut = out_payload_slice.read_val(size_of::<EntryOut>()).unwrap();

        Ok((entry_out, open_out))
    }

    pub fn fuse_getattr(&self, nodeid: u64) -> Result<FuseAttrOut, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<GetattrIn>()) as u32,
            FuseOpcode::Getattr.into(),
            unique,
            nodeid,
        );
        let getattr_in = GetattrIn::new(0);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, getattr_in, size_of::<FuseAttrOut>())?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();

        Ok(out_payload_slice.read_val(0).unwrap())
    }

    pub fn fuse_setattr(
        &self,
        nodeid: u64,
        setattr_in: SetattrIn,
    ) -> Result<FuseAttrOut, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<SetattrIn>()) as u32,
            FuseOpcode::Setattr.into(),
            unique,
            nodeid,
        );
        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, setattr_in, size_of::<FuseAttrOut>())?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();

        Ok(out_payload_slice.read_val(0).unwrap())
    }

    pub fn fuse_opendir(&self, nodeid: u64) -> Result<u64, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<OpenIn>()) as u32,
            FuseOpcode::Opendir.into(),
            unique,
            nodeid,
        );
        let open_in = OpenIn::new(0);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, open_in, size_of::<OpenOut>())?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let open_out: OpenOut = out_payload_slice.read_val(0).unwrap();

        Ok(open_out.fh)
    }

    pub fn fuse_readdir(
        &self,
        nodeid: u64,
        fh: u64,
        offset: u64,
        size: u32,
    ) -> Result<Vec<VirtioFsDirEntry>, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<ReadIn>()) as u32,
            FuseOpcode::Readdir.into(),
            unique,
            nodeid,
        );
        let read_in = ReadIn::new(fh, offset, size);

        let out_payload_size = size as usize;
        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, read_in, out_payload_size)?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        let out_header = self.check_reply(&out_header_slice, unique)?;
        let payload_len = (out_header.len as usize).saturating_sub(size_of::<OutHeader>());
        let payload_len = cmp::min(payload_len, out_payload_size);
        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();

        let mut payload = vec![0u8; payload_len];
        let mut payload_reader = out_payload_slice.reader().unwrap();
        payload_reader.limit(payload_len);
        payload_reader.read(&mut VmWriter::from(payload.as_mut_slice()));

        let mut entries = Vec::new();
        let mut pos = 0usize;

        while pos + size_of::<Dirent>() <= payload_len {
            let header: Dirent = out_payload_slice.read_val(pos).unwrap();
            if header.namelen == 0 {
                break;
            }

            let name_start = pos + size_of::<Dirent>();
            let name_end = name_start + header.namelen as usize;
            if name_end > payload_len {
                break;
            }

            if let Ok(name) = core::str::from_utf8(&payload[name_start..name_end]) {
                entries.push(VirtioFsDirEntry {
                    ino: header.ino,
                    offset: header.off,
                    type_: header.typ,
                    name: name.to_string(),
                });
            }

            let dirent_len = size_of::<Dirent>() + header.namelen as usize;
            let aligned = (dirent_len + 7) & !7;
            pos += aligned;
        }

        Ok(entries)
    }

    pub fn fuse_releasedir(&self, nodeid: u64, fh: u64) -> Result<(), VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<ReleaseIn>()) as u32,
            FuseOpcode::Releasedir.into(),
            unique,
            nodeid,
        );
        let release_in = ReleaseIn::new(fh, 0);

        let in_header_slice = self.prepare_in_header_buf(in_header)?;
        let in_payload_slice = self.prepare_in_payload_buf(release_in)?;
        let out_header_slice = self.prepare_out_header_buf()?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        Ok(())
    }

    pub fn fuse_readlink(&self, nodeid: u64) -> Result<String, VirtioDeviceError> {
        const MAX_READLINK_SIZE: usize = 4096;
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            size_of::<InHeader>() as u32,
            FuseOpcode::Readlink.into(),
            unique,
            nodeid,
        );

        let in_header_slice = self.prepare_in_header_buf(in_header)?;
        let out_header_slice = self.prepare_out_header_buf()?;
        let out_payload_slice = self.prepare_out_payload_buf(MAX_READLINK_SIZE)?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        let out_header = self.check_reply(&out_header_slice, unique)?;
        let payload_len = (out_header.len as usize).saturating_sub(size_of::<OutHeader>());
        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();

        let mut payload = vec![0u8; payload_len];
        let mut reader = out_payload_slice.reader().unwrap();
        reader.limit(payload_len);
        reader.read(&mut VmWriter::from(payload.as_mut_slice()));

        // Fuse readlink may include a trailing '\0'.
        let end = payload.iter().position(|b| *b == 0).unwrap_or(payload_len);
        let target = String::from_utf8_lossy(&payload[..end]).to_string();

        Ok(target)
    }

    pub fn fuse_link(
        &self,
        old_nodeid: u64,
        new_parent_nodeid: u64,
        new_name: &str,
    ) -> Result<EntryOut, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<LinkIn>() + new_name.len() + 1) as u32,
            FuseOpcode::Link.into(),
            unique,
            new_parent_nodeid,
        );
        let link_in = LinkIn::new(old_nodeid);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, link_in, size_of::<EntryOut>())?;
        let in_name_slice = self.prepare_in_name_buf(new_name)?;

        let selector = self.select_request_queue(new_parent_nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice, &in_name_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();

        Ok(out_payload_slice.read_val(0).unwrap())
    }

    pub fn fuse_open(&self, nodeid: u64, flags: u32) -> Result<OpenOut, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<OpenIn>()) as u32,
            FuseOpcode::Open.into(),
            unique,
            nodeid,
        );
        let open_in = OpenIn::new(flags);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, open_in, size_of::<OpenOut>())?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let open_out: OpenOut = out_payload_slice.read_val(0).unwrap();

        Ok(open_out)
    }

    pub fn fuse_release(&self, nodeid: u64, fh: u64, flags: u32) -> Result<(), VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<ReleaseIn>()) as u32,
            FuseOpcode::Release.into(),
            unique,
            nodeid,
        );
        let release_in = ReleaseIn::new(fh, flags);

        let in_header_slice = self.prepare_in_header_buf(in_header)?;
        let in_payload_slice = self.prepare_in_payload_buf(release_in)?;
        let out_header_slice = self.prepare_out_header_buf()?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        Ok(())
    }

    pub fn fuse_lseek(
        &self,
        nodeid: u64,
        fh: u64,
        offset: i64,
        whence: u32,
    ) -> Result<i64, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<LseekIn>()) as u32,
            FuseOpcode::Lseek.into(),
            unique,
            nodeid,
        );
        let lseek_in = LseekIn::new(fh, offset, whence);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, lseek_in, size_of::<LseekOut>())?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let lseek_out: LseekOut = out_payload_slice.read_val(0).unwrap();

        Ok(lseek_out.offset)
    }

    pub fn fuse_read(
        &self,
        nodeid: u64,
        fh: u64,
        offset: u64,
        size: u32,
    ) -> Result<Vec<u8>, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<ReadIn>()) as u32,
            FuseOpcode::Read.into(),
            unique,
            nodeid,
        );
        let read_in = ReadIn::new(fh, offset, size);

        let out_payload_size = size as usize;
        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, read_in, out_payload_size)?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        let out_header = self.check_reply(&out_header_slice, unique)?;

        let payload_len = (out_header.len as usize).saturating_sub(size_of::<OutHeader>());
        let payload_len = cmp::min(payload_len, out_payload_size);
        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();

        let mut content = vec![0u8; payload_len];
        let mut reader = out_payload_slice.reader().unwrap();
        reader.limit(payload_len);
        reader.read(&mut VmWriter::from(content.as_mut_slice()));

        Ok(content)
    }

    pub fn fuse_write(
        &self,
        nodeid: u64,
        fh: u64,
        offset: u64,
        data: &[u8],
    ) -> Result<usize, VirtioDeviceError> {
        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<WriteIn>() + data.len()) as u32,
            FuseOpcode::Write.into(),
            unique,
            nodeid,
        );
        let write_in = WriteIn::new(fh, offset, data.len() as u32);

        let (in_header_slice, in_payload_slice, out_header_slice, out_payload_slice) =
            self.prepare_request_slices(in_header, write_in, size_of::<WriteOut>())?;

        let in_data_slice = self.prepare_in_data_buf(data)?;

        let selector = self.select_request_queue(nodeid);
        self.submit_request_and_wait(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice, &in_data_slice],
            &[&out_header_slice, &out_payload_slice],
        )?;

        self.check_reply(&out_header_slice, unique)?;

        out_payload_slice
            .mem_obj()
            .sync_from_device(out_payload_slice.offset().clone())
            .unwrap();
        let write_out: WriteOut = out_payload_slice.read_val(0).unwrap();

        Ok(write_out.size as usize)
    }

    pub fn fuse_forget(&self, nodeid: u64, nlookup: u64) -> Result<(), VirtioDeviceError> {
        if nodeid == FUSE_ROOT_ID || nlookup == 0 {
            return Ok(());
        }

        let unique = self.alloc_unique();

        let in_header = InHeader::new(
            (size_of::<InHeader>() + size_of::<ForgetIn>()) as u32,
            FuseOpcode::Forget.into(),
            unique,
            nodeid,
        );
        let forget_in = ForgetIn::new(nlookup);

        let in_header_slice = self.prepare_in_header_buf(in_header)?;
        let in_payload_slice = self.prepare_in_payload_buf(forget_in)?;

        let selector = QueueSelector::Hiprio;
        let _ = self.submit_request(
            selector,
            unique,
            &[&in_header_slice, &in_payload_slice],
            &[],
        )?;

        Ok(())
    }
}
