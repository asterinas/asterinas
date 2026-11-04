// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};
use core::ops::Range;

use ostd::{mm::Paddr, sync::Mutex};
use tpm_core::{
    BootErr, ChipLink, Limits, Tis, Xfer, bring_up,
    chip::{ChipTransport, CtxIo},
    cmd::SU_CLEAR,
    module::{IoErr, Space},
    rewrite::read_be32,
    tis::TisErr,
};

use crate::{
    extcrypto::check_abi,
    mmio::TisMmio,
    space_io::{
        CcTable, SPACE_BUF, TPMA_CC_CHANDLES_MASK, TPMA_CC_CHANDLES_SHIFT, XmitErr, command_code,
        space_transmit,
    },
};

pub const TPM_TIS_BASE: Paddr = 0xFED4_0000;
pub const TPM_TIS_SIZE: usize = 0x5000;

pub enum TpmInitErr {
    Mmio(TisErr),
    Boot(BootErr),
    Commands(IoErr),
}

struct TpmChipState {
    io: CtxIo<ChipLink<TisMmio>>,
    work_ctx: Vec<u8>,
    work_ses: Vec<u8>,
}

pub struct TpmDevice {
    inner: Mutex<TpmChipState>,
    limits: Limits,
    cc_table: CcTable,
}

impl TpmDevice {
    pub fn exec(&self, cmd: &[u8], rsp: &mut [u8]) -> Result<usize, IoErr> {
        let mut chip = self.inner.lock();
        chip.io.exec_raw(cmd, rsp)
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn cc_table(&self) -> &CcTable {
        &self.cc_table
    }

    pub fn transmit_space(
        &self,
        space: &mut Space,
        ctx_buf: &mut [u8],
        ses_buf: &mut [u8],
        cmd: &mut [u8],
        cmd_len: usize,
        rsp: &mut [u8],
    ) -> Result<usize, XmitErr> {
        let mut chip = self.inner.lock();
        let TpmChipState {
            io,
            work_ctx,
            work_ses,
        } = &mut *chip;
        space_transmit(
            space,
            io,
            &self.cc_table,
            ctx_buf,
            ses_buf,
            work_ctx,
            work_ses,
            cmd,
            cmd_len,
            rsp,
        )
    }

    pub fn close_space(&self, space: &mut Space) {
        let mut chip = self.inner.lock();
        let transaction = space.begin();
        transaction.abort(&mut chip.io);
    }
}

const TPM_CAP_COMMANDS: u32 = 0x0000_0002;
const TPM_CAP_TPM_PROPERTIES: u32 = 0x0000_0006;
const TPM_PT_TOTAL_COMMANDS: u32 = 0x0000_0129;
const TPM_CC_FIRST: u32 = 0x0000_011f;
const TPM_CC_CONTEXT_SAVE: u32 = 0x0000_0162;
const TPM_CC_FLUSH_CONTEXT: u32 = 0x0000_0165;
const MAX_NR_COMMANDS: usize = 0x000f_ffff;
const CC_PAGE_COMMANDS: usize = 256;
const GET_CAPABILITY_COMMAND_SIZE: usize = 22;
const GET_CAPABILITY_RESPONSE_PREFIX: usize = 19;
const COMMAND_ATTR_SIZE: usize = 4;

fn get_total_commands<T: ChipTransport>(chip: &mut T) -> Result<usize, IoErr> {
    let mut command = [0u8; GET_CAPABILITY_COMMAND_SIZE];
    let mut response = [0u8; 64];

    command[0..2].copy_from_slice(&0x8001u16.to_be_bytes());
    command[2..6].copy_from_slice(&(GET_CAPABILITY_COMMAND_SIZE as u32).to_be_bytes());
    command[6..10].copy_from_slice(&0x0000_017au32.to_be_bytes());
    command[10..14].copy_from_slice(&TPM_CAP_TPM_PROPERTIES.to_be_bytes());
    command[14..18].copy_from_slice(&TPM_PT_TOTAL_COMMANDS.to_be_bytes());
    command[18..22].copy_from_slice(&1u32.to_be_bytes());

    let response_len = chip.exec(&command, &mut response)?;
    if response_len != 27
        || read_be32(&response, 2) as usize != response_len
        || read_be32(&response, 6) != 0
        || response[10] > 1
        || read_be32(&response, 11) != TPM_CAP_TPM_PROPERTIES
        || read_be32(&response, 15) != 1
        || read_be32(&response, 19) != TPM_PT_TOTAL_COMMANDS
    {
        return Err(IoErr::Protocol);
    }

    let nr_commands = read_be32(&response, 23) as usize;
    if nr_commands == 0 || nr_commands > MAX_NR_COMMANDS {
        return Err(IoErr::Protocol);
    }
    Ok(nr_commands)
}

fn build_cc_table<T: ChipTransport>(chip: &mut T) -> Result<CcTable, IoErr> {
    let nr_commands = get_total_commands(chip)?;
    let mut table = CcTable::with_capacity(nr_commands)?;
    let mut property = TPM_CC_FIRST;
    let mut command = [0u8; GET_CAPABILITY_COMMAND_SIZE];
    let mut response = [0u8; 4096];

    loop {
        let remaining = nr_commands
            .checked_sub(table.len())
            .filter(|remaining| *remaining > 0)
            .ok_or(IoErr::Protocol)?;
        let request_count = core::cmp::min(remaining, CC_PAGE_COMMANDS);

        command.fill(0);
        command[0..2].copy_from_slice(&0x8001u16.to_be_bytes());
        command[2..6].copy_from_slice(&(GET_CAPABILITY_COMMAND_SIZE as u32).to_be_bytes());
        command[6..10].copy_from_slice(&0x0000_017au32.to_be_bytes());
        command[10..14].copy_from_slice(&TPM_CAP_COMMANDS.to_be_bytes());
        command[14..18].copy_from_slice(&property.to_be_bytes());
        command[18..22].copy_from_slice(&(request_count as u32).to_be_bytes());

        let response_len = chip.exec(&command, &mut response)?;
        if response_len < GET_CAPABILITY_RESPONSE_PREFIX {
            return Err(IoErr::Protocol);
        }
        let declared_len = read_be32(&response, 2) as usize;
        let response_code = read_be32(&response, 6);
        let capability = read_be32(&response, 11);
        let count = read_be32(&response, 15) as usize;
        let attrs_len = count
            .checked_mul(COMMAND_ATTR_SIZE)
            .and_then(|len| GET_CAPABILITY_RESPONSE_PREFIX.checked_add(len))
            .ok_or(IoErr::Protocol)?;
        if response[10] > 1
            || response_code != 0
            || capability != TPM_CAP_COMMANDS
            || declared_len != response_len
            || attrs_len != response_len
            || count > request_count
            || count > remaining
            || (count == 0 && response[10] != 0)
        {
            return Err(IoErr::Protocol);
        }

        let mut previous_cc = None;
        let mut last_cc = None;
        for index in 0..count {
            let mut attr = read_be32(
                &response,
                GET_CAPABILITY_RESPONSE_PREFIX + index * COMMAND_ATTR_SIZE,
            );
            let cc = command_code(attr);
            if cc < property || previous_cc.is_some_and(|previous| cc <= previous) {
                return Err(IoErr::Protocol);
            }
            previous_cc = Some(cc);
            last_cc = Some(cc);

            // The TPM command attributes report ContextSave and FlushContext
            // with no command handles even though their first parameter is a
            // handle. Linux applies the same correction before using the
            // attributes for resource-manager handle translation.
            if cc == TPM_CC_CONTEXT_SAVE || cc == TPM_CC_FLUSH_CONTEXT {
                attr &= !(TPMA_CC_CHANDLES_MASK << TPMA_CC_CHANDLES_SHIFT);
                attr |= 1 << TPMA_CC_CHANDLES_SHIFT;
            }

            if table.len() >= nr_commands {
                return Err(IoErr::Protocol);
            }
            table.push(attr);
        }

        if response[10] == 0 {
            if table.len() != nr_commands {
                return Err(IoErr::Protocol);
            }
            break;
        }
        if count == 0 || table.len() >= nr_commands {
            return Err(IoErr::Protocol);
        }
        property = last_cc
            .ok_or(IoErr::Protocol)?
            .checked_add(1)
            .ok_or(IoErr::Protocol)?;
    }

    if table.len() != nr_commands {
        return Err(IoErr::Protocol);
    }
    Ok(table)
}

pub fn probe() -> Result<TpmDevice, TpmInitErr> {
    let abi_ok = check_abi();
    if !abi_ok {
        ostd::warn!("tpm: host crypto ABI mismatch, boot will abort");
    }

    let phys: Range<Paddr> = TPM_TIS_BASE..TPM_TIS_BASE + TPM_TIS_SIZE;
    let mmio = TisMmio::acquire(phys).map_err(TpmInitErr::Mmio)?;

    let tis = Tis::new(mmio, 0);
    let x = Xfer::new(tis);
    let (x, limits) = bring_up(x, SU_CLEAR, abi_ok).map_err(TpmInitErr::Boot)?;

    let mut chip = ChipLink::new(x);
    let cc_table = build_cc_table(&mut chip).map_err(TpmInitErr::Commands)?;

    ostd::info!("tpm: ready");
    Ok(TpmDevice {
        inner: Mutex::new(TpmChipState {
            io: CtxIo::new(chip),
            work_ctx: vec![0u8; SPACE_BUF],
            work_ses: vec![0u8; SPACE_BUF],
        }),
        limits,
        cc_table,
    })
}
