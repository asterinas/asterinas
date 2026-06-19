use x86::msr::*;

use crate::{
    arch::vm::context::{GuestContext, sanitize_apic_base_for_vcpu},
    prelude::*,
    sync::Mutex,
};

pub(crate) fn emulate_msrrw(context: &Mutex<GuestContext>, is_write: bool) -> Result<()> {
    let mut context = context.lock();
    let msr_index = context.arch().gpr(2) as u32;

    if is_write {
        let msr_value =
            (context.arch().gpr(0) as u32 as u64) | ((context.arch().gpr(3) as u32 as u64) << 32);

        match msr_index {
            TSC => {
                let raw_tsc = crate::arch::read_tsc();
                context.tsc_offset = msr_value as i64 - raw_tsc as i64;
            }
            IA32_TSC_ADJUST => {
                let delta = msr_value as i64 - context.arch().msr(IA32_TSC_ADJUST) as i64;
                context.arch_mut().set_msr(IA32_TSC_ADJUST, msr_value);
                context.tsc_offset += delta;
            }
            IA32_APIC_BASE => {
                let apic_base = sanitize_apic_base_for_vcpu(msr_value, context.cpu_config.vcpu_id);
                context.arch_mut().set_msr(IA32_APIC_BASE, apic_base);
            }
            IA32_EFER => {
                context.arch_mut().set_efer(msr_value);
            }
            IA32_BIOS_SIGN_ID => {}
            IA32_MISC_ENABLE => {}
            IA32_TSC_DEADLINE => {
                context.arch_mut().set_msr(IA32_TSC_DEADLINE, msr_value);
                context.tsc_deadline = (msr_value != 0).then_some(msr_value);
                return Ok(());
            }
            _ => {
                context.arch_mut().set_msr(msr_index, msr_value);
            }
        }

        return Ok(());
    }

    // is read
    let msr_value = match msr_index {
        TSC => context.guest_tsc(),
        _ => context.arch().msr(msr_index),
    };

    context.arch_mut().set_gpr(0, 8, msr_value as u32 as u64);
    context.arch_mut().set_gpr(3, 8, msr_value >> 32);
    Ok(())
}
