use crate::{
    Error,
    arch::vm::{context::GuestContext, vmx::VmcsReadOnlyNW},
    prelude::*,
    sync::Mutex,
};

pub(crate) fn emulate_cr_access(context: &Mutex<GuestContext>) -> Result<()> {
    let qualification = VmcsReadOnlyNW::EXIT_QUALIFICATION
        .read()
        .map_err(Error::from)?;
    let cr_index = (qualification & 0xF) as u8;
    let access = ((qualification >> 4) & 0b11) as u8;
    let gpr_index = ((qualification >> 8) & 0xF) as u8;
    let gpr_index = map_exit_qualification_gpr_index_to_common_gpr_index(gpr_index);
    let mut context = context.lock();

    match access {
        // write
        0 => {
            let value = context.arch().gpr(gpr_index);
            match cr_index {
                // A value different from the shadow was written to the masked bit.
                0 => context.arch_mut().write_cr0(value),
                2 => context.arch_mut().set_cr2(value),
                3 => context.arch_mut().set_cr3(value),
                4 => context.arch_mut().write_cr4(value),
                other => warn!("rustshyper: ignoring guest write to CR{}", other),
            }
        }
        // read
        1 => {
            let value = match cr_index {
                0 => context.arch().cr0(),
                2 => context.arch().cr2(),
                3 => context.arch().cr3(),
                4 => context.arch().cr4(),
                other => {
                    warn!("rustshyper: ignoring guest read from CR{}", other);
                    0
                }
            };
            context.arch_mut().set_gpr(gpr_index, 8, value);
        }
        other => {
            warn!("rustshyper: unsupported CR access type {}", other);
        }
    }

    Ok(())
}

fn map_exit_qualification_gpr_index_to_common_gpr_index(index: u8) -> u8 {
    // The exit qualification field in the VM-Exit Information Fields of the VMCS
    // uses a different numbering method for gpr than that used in this project.
    // Therefore, a mapping layer is needed.
    //
    // The encoding method of the "exit qualification" field:
    // Intel® 64 and IA-32 Architectures Software Developer’s Manual
    // Volume 3, 29.2.1 Basic VM-Exit Information
    match index {
        0 => 0,
        1 => 2,
        2 => 3,
        3 => 1,
        4 => 7,
        5 => 6,
        6 => 4,
        7 => 5,
        other => other,
    }
}
