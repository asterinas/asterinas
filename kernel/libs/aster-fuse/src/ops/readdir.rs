// SPDX-License-Identifier: MPL-2.0

//! `FUSE_READDIR` reads directory entries from an open directory handle.
//!
//! The request body reuses [`ReadReq`] with the directory handle, offset, and
//! maximum byte count. The reply body is a sequence of [`Dirent`] headers
//! followed by 8-byte-padded names, and the operation returns decoded
//! [`FuseDirEntry`] values.

use alloc::{string::ToString, vec::Vec};

use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_bytes;
use crate::{
    DirOffset, Dirent, DirentType, FuseDirEntry, FuseError, FuseOpcode, FuseOperation, FuseResult,
    ReadReq, ReplyExpectation,
};

/// The maximum directory entry name bytes decoded in one FUSE record.
const MAX_NAME_LEN: usize = 1024;

pub struct ReaddirOperation {
    read_req: ReadReq,
}

impl ReaddirOperation {
    pub fn new(read_req: ReadReq) -> Self {
        Self { read_req }
    }

    pub fn parse_entries(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Vec<FuseDirEntry>> {
        reader.limit(payload_len);

        let mut name_buf = [0u8; MAX_NAME_LEN];
        let capacity = reader.remain() / size_of::<Dirent>();
        let mut entries = Vec::with_capacity(capacity);
        while reader.remain() >= size_of::<Dirent>() {
            let header = reader
                .read_val::<Dirent>()
                .map_err(|_| FuseError::BufferTooSmall)?;
            if header.namelen() == 0 {
                return Err(FuseError::MalformedResponse);
            }

            let namelen = header.namelen() as usize;
            if namelen > name_buf.len() || namelen > reader.remain() {
                return Err(FuseError::MalformedResponse);
            }
            read_bytes(reader, &mut name_buf[..namelen])?;

            let name = core::str::from_utf8(&name_buf[..namelen])
                .map_err(|_| FuseError::MalformedResponse)?;
            entries.push(FuseDirEntry::new(
                header.ino(),
                DirOffset::new(header.off()),
                DirentType::try_from(header.typ()).unwrap_or(DirentType::Unknown),
                name.to_string(),
            ));

            // Each dirent is padded to 8-byte alignment.
            let dirent_len = size_of::<Dirent>() + namelen;
            let padded = (dirent_len + 7) & !7;
            let pad = padded - dirent_len;
            if pad > reader.remain() {
                return Err(FuseError::MalformedResponse);
            }
            reader.skip(pad);
        }

        if reader.remain() != 0 {
            return Err(FuseError::MalformedResponse);
        }

        Ok(entries)
    }
}

impl FuseOperation for ReaddirOperation {
    type Output = Vec<FuseDirEntry>;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Readdir
    }

    fn body_len(&self) -> usize {
        size_of::<ReadReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.read_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(self.read_req.size() as usize)
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        Self::parse_entries(payload_len, reader)
    }
}
