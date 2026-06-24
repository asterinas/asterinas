use crate::{arch::vm::context::GuestContext, prelude::*, sync::Mutex};

pub(crate) fn emulate_msrrw(context: &Mutex<GuestContext>, is_write: bool) -> Result<()> {
    let mut context = context.lock();
    let msr_index = context.arch().gpr(2) as u32;

    if is_write {
        let msr_value =
            (context.arch().gpr(0) as u32 as u64) | ((context.arch().gpr(3) as u32 as u64) << 32);

        if !context.write_msr(msr_index, msr_value) {
            error!("set_msr: msr {:x} not impl.", msr_index);
        }

        return Ok(());
    }

    // is read
    let msr_value = context.read_msr(msr_index).unwrap_or_else(|| {
        error!("get unknown msr {:x}, return 0.", msr_index);
        0
    });

    context.arch_mut().set_gpr(0, 8, msr_value as u32 as u64);
    context.arch_mut().set_gpr(3, 8, msr_value >> 32);
    Ok(())
}
