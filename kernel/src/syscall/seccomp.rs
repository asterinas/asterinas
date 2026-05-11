use alloc::vec::Vec;

use cbpf_opcodes::ClassicBpfOpcode::{self, *};
use ostd::task::{
    Task,
    seccomp::{
        FilterBlock, RawFilterBlock, SeccompFilterBlock, SeccompFilterProg, SeccompMode, SeccompOp,
        UserspaceFilterMeta,
        cbpf_opcodes::{self, AncOps, BPF_MAXINS, BPF_MEMWORDS, SKF_AD_OFF, SeccompOpcode},
    },
};

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_seccomp(op: u64, flags: u32, uargs: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let op = match op {
        0 => SeccompOp::SECCOMP_SET_MODE_STRICT,
        1 => SeccompOp::SECCOMP_SET_MODE_FILTER,
        _ => Err(Error::new(Errno::EINVAL))?,
    };

    do_seccomp(op, flags, uargs, ctx)
}

fn do_seccomp(op: SeccompOp, flags: u32, uargs: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    // what if the process already has seccomp enabled? we'll need to manage a tree structure holding multiple filters for threads and their forks
    // I believe we don't need to check if user changes from strict mode to filter or vice versa, because:
    // 1. strict mode prevent seccomp syscall from being executed
    // 2. filter to strict should be allowed, but we will need to drop the Arc reference to so there's no memory leak

    let res: i64 = match op {
        SeccompOp::SECCOMP_SET_MODE_STRICT => {
            if flags != 0 || uargs != 0 {
                return Err(Error::new(Errno::EINVAL));
            }
            seccomp_set_mode_strict(ctx)
        }
        SeccompOp::SECCOMP_SET_MODE_FILTER => seccomp_set_mode_filter(flags, uargs, ctx),
    }
    .map_err(|err: Error| match err.error() {
        _ => err,
    })?;

    Ok(SyscallReturn::Return(res as _))
}

fn seccomp_set_mode_strict(ctx: &Context) -> Result<i64> {
    // Linux does mitigations here
    seccomp_assign_mode(ctx.task, SeccompMode::SECCOMP_MODE_STRICT, 0)
}

fn seccomp_assign_mode(current: &Task, mode: SeccompMode, _flags: u64) -> Result<i64> {
    // Linux does mitigations here
    current.seccomp.lock().mode = mode;
    Ok(0)
}

// currently can add only the first filter to a thread
// has not been tested as of yet!
fn seccomp_set_mode_filter(flags: u32, uargs: Vaddr, ctx: &Context) -> Result<i64> {
    let filter_meta: UserspaceFilterMeta = ctx
        .user_space()
        .vmar()
        .vm_space()
        .reader(uargs, size_of::<UserspaceFilterMeta>())
        .unwrap()  // TODO remove unwrap()
        .read_val()
        .unwrap(); // TODO remove unwrap()

    let filter_len = filter_meta.user_buf_len;
    let mut insns: Vec<FilterBlock> = Vec::with_capacity(filter_len);

    for i in 0..filter_len {
        let raw_instruction = ctx
            .user_space()
            .vmar()
            .vm_space()
            .reader(
                filter_meta.user_buf_ptr + size_of::<RawFilterBlock>() * i,
                size_of::<RawFilterBlock>(),
            )
            .unwrap()  // TODO remove unwrap()
            .read_val::<RawFilterBlock>()
            .unwrap(); // TODO remove unwrap()

        if let Ok(instruction) = raw_instruction.try_into() {
            insns[i] = instruction;
        } else {
            return Err(Error::new(Errno::EINVAL));
        }
    }

    verify_cbpf(&insns)?;

    let insns = insns
        .into_iter()
        .map(verify_and_map_seccomp)
        .collect::<Option<Vec<_>>>()
        .ok_or(Error::new(Errno::EINVAL))?;

    ctx.task.seccomp.lock().leaf_filter = Some(Arc::new(SeccompFilterProg {
        ins: insns.into_boxed_slice(),
        prev: None,
    }));
    seccomp_assign_mode(ctx.task, SeccompMode::SECCOMP_MODE_FILTER, 0);
    Err(Error::new(Errno::EINVAL))
}

// https://elixir.bootlin.com/linux/v6.18/source/net/core/filter.c#L1081
fn verify_cbpf(insns: &Vec<FilterBlock>) -> Result<()> {
    let len = insns.len();

    // https://elixir.bootlin.com/linux/v6.18/source/net/core/filter.c#L1056
    if len == 0 || len > BPF_MAXINS {
        return Err(Error::new(Errno::EINVAL));
    }

    for (i, FilterBlock { code, jt, jf, k }) in insns.iter().enumerate() {
        let valid: bool = match code {
            ALU_DIV_K | ALU_MOD_K => {
                if *k == 0 {
                    false
                } else {
                    true
                }
            }
            ALU_LSH_K | ALU_RSH_K => {
                if *k >= 32u32 {
                    false
                } else {
                    true
                }
            }
            LD_MEM | LDX_MEM | ST | STX => {
                if *k >= BPF_MEMWORDS {
                    false
                } else {
                    true
                }
            }
            JMP_JA => {
                if *k >= (len - i - 1) as u32 {
                    false
                } else {
                    true
                }
            }
            JMP_JEQ_K | JMP_JEQ_X | JMP_JGE_K | JMP_JGE_X | JMP_JGT_K | JMP_JGT_X | JMP_JSET_K
            | JMP_JSET_X => {
                if i + (*jt as usize) + 1 >= len || i + (*jf as usize) + 1 >= len {
                    false
                } else {
                    true
                }
            }
            LD_W_ABS | LD_H_ABS | LD_B_ABS => {
                *k < SKF_AD_OFF || AncOps::try_from(k - SKF_AD_OFF).is_ok()
            }
            _ => true,
        };

        if !valid {
            return Err(Error::new(Errno::EINVAL));
        }
    }

    match ClassicBpfOpcode::from(insns[len - 1].code) {
        ClassicBpfOpcode::RET_K | ClassicBpfOpcode::RET_A => return check_load_and_stores(insns),
        _ => (),
    }

    Err(Error::new(Errno::EINVAL))
}

// does it (and Linux) check for infinite loops? if not, does Linux allow infinite loops in cbpf?
// if that's the case, should our verifier be more strict?
// https://elixir.bootlin.com/linux/v6.18/source/kernel/seccomp.c#L278
fn check_load_and_stores(filter: &Vec<FilterBlock>) -> Result<()> {
    let mut memvalid: u16 = 0;
    let mut masks = vec![0xffffu16; filter.len()];

    for (i, ins) in filter.iter().enumerate() {
        match ins.code {
            ST | STX => memvalid |= 1 << ins.k,
            LD_MEM | LDX_MEM => {
                if (memvalid & (1 << ins.k)) == 0 {
                    return Err(Error::new(Errno::EINVAL));
                }
            }
            JMP_JA => {
                masks[i + 1 + (ins.k as usize)] &= memvalid;
                memvalid = 0xff;
            }
            JMP_JEQ_K | JMP_JEQ_X | JMP_JGE_K | JMP_JGE_X | JMP_JGT_K | JMP_JGT_X | JMP_JSET_K
            | JMP_JSET_X => {
                masks[i + 1 + (ins.jt as usize)] &= memvalid;
                masks[i + 1 + (ins.jf as usize)] &= memvalid;
                memvalid = 0xff;
            }
            _ => (),
        }
    }
    Ok(())
}

// https://elixir.bootlin.com/linux/v6.18/source/kernel/seccomp.c#L278
fn verify_and_map_seccomp(ins: FilterBlock) -> Option<SeccompFilterBlock> {
    use ClassicBpfOpcode::*;
    let code: SeccompOpcode;
    let mut k = ins.k;

    match ins.code {
        LD_W_ABS => {
            if k >= 64 /*size_of::<seccomp_input_data>() TODO*/ || k & 3 != 0 {
                return None;
            }
            code = SeccompOpcode::LDX_W_ABS; // TODO allowed only in seccomp, reflect in data model
        }
        LD_W_LEN => {
            code = SeccompOpcode::cBPF(LD_IMM);
            k = 64 /*size_of::<seccomp_input_data>()*/;
        }
        LDX_W_LEN => {
            code = SeccompOpcode::cBPF(LDX_IMM);
            k = 64 /*size_of::<seccomp_input_data>()*/;
        }
        RET_K | RET_A | ALU_ADD_K | ALU_ADD_X | ALU_SUB_K | ALU_SUB_X | ALU_MUL_K | ALU_MUL_X
        | ALU_DIV_K | ALU_DIV_X | ALU_AND_K | ALU_AND_X | ALU_OR_K | ALU_OR_X | ALU_XOR_K
        | ALU_XOR_X | ALU_LSH_K | ALU_LSH_X | ALU_RSH_K | ALU_RSH_X | ALU_NEG | LD_IMM
        | LDX_IMM | MISC_TAX | MISC_TXA | LD_MEM | LDX_MEM | ST | STX | JMP_JA | JMP_JEQ_K
        | JMP_JEQ_X | JMP_JGE_K | JMP_JGE_X | JMP_JGT_K | JMP_JGT_X | JMP_JSET_K | JMP_JSET_X => {
            code = SeccompOpcode::cBPF(ins.code)
        }
        _ => return None,
    }

    Some(SeccompFilterBlock {
        code,
        jt: ins.jt,
        jf: ins.jf,
        k,
    })
}
