// SPDX-License-Identifier: MPL-2.0

//! TPM resource-space isolation for character-device commands.

#![allow(dead_code)]

use alloc::vec::Vec;

use tpm_core::{
    chip::{ChipTransport, CtxIo},
    module::{ContextIo, IoErr, Space},
    rewrite::{
        HEADER_SIZE, HeaderOutcome, SpaceErr, map_capability_handles, map_command_handles,
        map_response_handle, read_be32,
    },
};

const CC_GET_CAPABILITY: u32 = 0x0000_017A;

#[derive(Clone, Copy)]
pub struct CcAttrs {
    pub nr_chandles: usize,
    pub has_rhandle: bool,
}

pub(super) const TPMA_CC_COMMAND_INDEX_MASK: u32 = 0x0000_ffff;
pub(super) const TPMA_CC_VENDOR: u32 = 1 << 29;
pub(super) const TPMA_CC_CHANDLES_SHIFT: u32 = 25;
pub(super) const TPMA_CC_CHANDLES_MASK: u32 = 0x7;
pub(super) const TPMA_CC_RHANDLE: u32 = 1 << 28;

pub(super) fn command_code(attr: u32) -> u32 {
    (attr & TPMA_CC_COMMAND_INDEX_MASK) | (attr & TPMA_CC_VENDOR)
}

/// Per-open resource-space backing capacity for object and session contexts.
pub const SPACE_BUF: usize = 16384;

// Discover attributes at boot to support the chip's command set without a fixed table.
pub struct CcTable {
    attrs: Vec<u32>,
}

impl CcTable {
    pub(super) fn with_capacity(capacity: usize) -> Result<Self, IoErr> {
        let mut attrs = Vec::new();
        attrs
            .try_reserve_exact(capacity)
            .map_err(|_| IoErr::NoSpace)?;
        Ok(Self { attrs })
    }

    pub(super) fn push(&mut self, attr: u32) {
        self.attrs.push(attr);
    }

    pub fn lookup(&self, cc: u32) -> Option<CcAttrs> {
        self.attrs
            .iter()
            .copied()
            .find(|attr| command_code(*attr) == cc)
            .map(|attr| CcAttrs {
                nr_chandles: ((attr >> TPMA_CC_CHANDLES_SHIFT) & TPMA_CC_CHANDLES_MASK) as usize,
                has_rhandle: attr & TPMA_CC_RHANDLE != 0,
            })
    }

    pub fn len(&self) -> usize {
        self.attrs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmitErr {
    Malformed,
    Unsupported,
    BadHandle,
    NoSlots,
    Io(IoErr),
}

impl From<SpaceErr> for XmitErr {
    fn from(e: SpaceErr) -> Self {
        match e {
            SpaceErr::Malformed => XmitErr::Malformed,
            SpaceErr::BadHandle => XmitErr::BadHandle,
        }
    }
}

impl From<IoErr> for XmitErr {
    fn from(e: IoErr) -> Self {
        XmitErr::Io(e)
    }
}

/// Uses persistent context backups and per-request command and response buffers.
#[expect(clippy::too_many_arguments)]
pub fn space_transmit<T: ChipTransport>(
    space: &mut Space,
    io: &mut CtxIo<T>,
    cc_table: &CcTable,
    ctx_buf: &mut [u8],
    ses_buf: &mut [u8],
    work_ctx: &mut [u8],
    work_ses: &mut [u8],
    cmd: &mut [u8],
    cmd_len: usize,
    rsp: &mut [u8],
) -> Result<usize, XmitErr> {
    if cmd_len > cmd.len() {
        return Err(XmitErr::Malformed);
    }
    if cmd_len < HEADER_SIZE {
        return Err(XmitErr::Malformed);
    }
    if work_ctx.len() != ctx_buf.len() || work_ses.len() != ses_buf.len() {
        return Err(XmitErr::Io(IoErr::NoSpace));
    }
    let cc = read_be32(cmd, 6);
    let attrs = cc_table.lookup(cc).ok_or(XmitErr::Unsupported)?;
    if cmd_len < HEADER_SIZE + 4 * attrs.nr_chandles {
        return Err(XmitErr::Malformed);
    }

    let mut txn = space.begin();
    // Commit the table and context buffers together only after all steps succeed.
    work_ctx.copy_from_slice(ctx_buf);
    work_ses.copy_from_slice(ses_buf);

    if let Err(e) = tpm_core::module::load_space(txn.table(), io, work_ctx, work_ses) {
        txn.abort(io);
        return Err(e.into());
    }

    if let Err(e) = map_command_handles(&*txn.table(), attrs.nr_chandles, &mut cmd[..cmd_len]) {
        txn.abort(io);
        return Err(e.into());
    }

    let n = match io.exec_raw(&cmd[..cmd_len], rsp) {
        Ok(n) => n,
        Err(e) => {
            txn.abort(io);
            return Err(e.into());
        }
    };
    if n < HEADER_SIZE {
        txn.abort(io);
        return Err(XmitErr::Io(IoErr::Protocol));
    }
    let response_code = read_be32(&rsp[..n], 6);

    // TPM errors can omit the response handle, even when the command returns one.
    let outcome = if response_code == 0 && attrs.has_rhandle {
        if n < HEADER_SIZE + 4 {
            txn.abort(io);
            return Err(XmitErr::Io(IoErr::Protocol));
        }
        map_response_handle(txn.table(), true, &mut rsp[..n])
    } else {
        HeaderOutcome::NoHandle
    };
    if let HeaderOutcome::OutOfSlots { flush } = outcome {
        // Release the untracked handle before discarding the working space.
        io.flush(flush);
        txn.abort(io);
        return Err(XmitErr::NoSlots);
    }

    let is_cap_query = cc == CC_GET_CAPABILITY;
    let n = if response_code == 0 && is_cap_query {
        match map_capability_handles(&*txn.table(), true, rsp, n) {
            Ok(n) => n,
            Err(_) => {
                txn.abort(io);
                return Err(XmitErr::Io(IoErr::Protocol));
            }
        }
    } else {
        n
    };

    if let Err(e) = tpm_core::module::save_space(txn.table(), io, work_ctx, work_ses) {
        txn.abort(io);
        return Err(e.into());
    }

    ctx_buf.copy_from_slice(work_ctx);
    ses_buf.copy_from_slice(work_ses);
    space.commit(txn);
    Ok(n)
}
