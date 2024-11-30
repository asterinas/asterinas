// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use align_ext::AlignExt;
use aster_rights::Full;
use ostd::mm::VmIo;
use xmas_elf::program::{self, ProgramHeader64};

use super::elf_file::Elf;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver, AT_FDCWD},
        path::Dentry,
    },
    prelude::*,
    process::{
        do_exit_group,
        process_vm::{AuxKey, AuxVec, ProcessVm},
        TermStatus,
    },
    vdso::{vdso_vmo, VDSO_VMO_SIZE},
    vm::{perms::VmPerms, util::duplicate_frame, vmar::Vmar, vmo::VmoRightsOp},
};

/// Loads elf to the process vm.   
///
/// This function will map elf segments and
/// initialize process init stack.
pub fn load_elf_to_vm(
    process_vm: &ProcessVm,
    file_header: &[u8],
    elf_file: Dentry,
    fs_resolver: &FsResolver,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<ElfLoadInfo> {
    let parsed_elf = Elf::parse_elf(file_header)?;

    let ldso = lookup_and_parse_ldso(&parsed_elf, file_header, fs_resolver)?;

    match init_and_map_vmos(process_vm, ldso, &parsed_elf, &elf_file) {
        Ok((entry_point, mut aux_vec)) => {
            // Map and set vdso entry.
            // Since vdso does not require being mapped to any specific address,
            // vdso is mapped after the elf file, heap and stack are mapped.
            if let Some(vdso_text_base) = map_vdso_to_vm(process_vm) {
                aux_vec
                    .set(AuxKey::AT_SYSINFO_EHDR, vdso_text_base as u64)
                    .unwrap();
            }

            process_vm.map_and_write_init_stack(argv, envp, aux_vec)?;

            let user_stack_top = process_vm.user_stack_top();
            Ok(ElfLoadInfo {
                entry_point,
                user_stack_top,
            })
        }
        Err(err) => {
            // Since the process_vm is in invalid state,
            // the process cannot return to user space again,
            // so `Vmar::clear` and `do_exit_group` are called here.
            // FIXME: sending a fault signal is an alternative approach.
            process_vm.root_vmar().clear().unwrap();

            // FIXME: `current` macro will be used in `do_exit_group`.
            // if the macro is used when creating the init process,
            // the macro will panic. This corner case should be handled later.
            // FIXME: how to set the correct exit status?
            do_exit_group(TermStatus::Exited(1));

            // The process will exit and the error code will be ignored.
            Err(err)
        }
    }
}

fn lookup_and_parse_ldso(
    elf: &Elf,
    file_header: &[u8],
    fs_resolver: &FsResolver,
) -> Result<Option<(Dentry, Elf)>> {
    let ldso_file = {
        let Some(ldso_path) = elf.ldso_path(file_header)? else {
            return Ok(None);
        };
        let fs_path = FsPath::new(AT_FDCWD, &ldso_path)?;
        fs_resolver.lookup(&fs_path)?
    };
    let ldso_elf = {
        let mut buf = Box::new([0u8; PAGE_SIZE]);
        let inode = ldso_file.inode();
        inode.read_bytes_at(0, &mut *buf)?;
        Elf::parse_elf(&*buf)?
    };
    Ok(Some((ldso_file, ldso_elf)))
}

fn load_ldso(root_vmar: &Vmar<Full>, ldso_file: &Dentry, ldso_elf: &Elf) -> Result<LdsoLoadInfo> {
    let map_addr = map_segment_vmos(ldso_elf, root_vmar, ldso_file)?;
    Ok(LdsoLoadInfo::new(
        ldso_elf.entry_point() + map_addr,
        map_addr,
    ))
}

fn init_and_map_vmos(
    process_vm: &ProcessVm,
    ldso: Option<(Dentry, Elf)>,
    parsed_elf: &Elf,
    elf_file: &Dentry,
) -> Result<(Vaddr, AuxVec)> {
    let root_vmar = process_vm.root_vmar();

    // After we clear process vm, if any error happens, we must call exit_group instead of return to user space.
    let ldso_load_info = if let Some((ldso_file, ldso_elf)) = ldso {
        Some(load_ldso(root_vmar, &ldso_file, &ldso_elf)?)
    } else {
        None
    };

    let elf_map_addr = map_segment_vmos(parsed_elf, root_vmar, elf_file)?;

    let aux_vec = {
        let ldso_base = ldso_load_info
            .as_ref()
            .map(|load_info| load_info.base_addr());
        init_aux_vec(parsed_elf, elf_map_addr, ldso_base)?
    };

    let entry_point = if let Some(ldso_load_info) = ldso_load_info {
        // Normal shared object
        ldso_load_info.entry_point()
    } else if parsed_elf.is_shared_object() {
        // ldso itself
        parsed_elf.entry_point() + elf_map_addr
    } else {
        // statically linked executable
        parsed_elf.entry_point()
    };

    Ok((entry_point, aux_vec))
}

pub struct LdsoLoadInfo {
    entry_point: Vaddr,
    base_addr: Vaddr,
}

impl LdsoLoadInfo {
    pub fn new(entry_point: Vaddr, base_addr: Vaddr) -> Self {
        Self {
            entry_point,
            base_addr,
        }
    }

    pub fn entry_point(&self) -> Vaddr {
        self.entry_point
    }

    pub fn base_addr(&self) -> Vaddr {
        self.base_addr
    }
}

pub struct ElfLoadInfo {
    entry_point: Vaddr,
    user_stack_top: Vaddr,
}

impl ElfLoadInfo {
    pub fn new(entry_point: Vaddr, user_stack_top: Vaddr) -> Self {
        Self {
            entry_point,
            user_stack_top,
        }
    }

    pub fn entry_point(&self) -> Vaddr {
        self.entry_point
    }

    pub fn user_stack_top(&self) -> Vaddr {
        self.user_stack_top
    }
}

/// Inits VMO for each segment and then map segment to root vmar
pub fn map_segment_vmos(elf: &Elf, root_vmar: &Vmar<Full>, elf_file: &Dentry) -> Result<Vaddr> {
    // all segments of the shared object must be mapped to a continuous vm range
    // to ensure the relative offset of each segment not changed.
    let base_addr = if elf.is_shared_object() {
        base_map_addr(elf, root_vmar)?
    } else {
        0
    };
    for program_header in &elf.program_headers {
        let type_ = program_header
            .get_type()
            .map_err(|_| Error::with_message(Errno::ENOEXEC, "parse program header type fails"))?;
        if type_ == program::Type::Load {
            check_segment_align(program_header)?;
            map_segment_vmo(program_header, elf_file, root_vmar, base_addr)?;
        }
    }
    Ok(base_addr)
}

fn base_map_addr(elf: &Elf, root_vmar: &Vmar<Full>) -> Result<Vaddr> {
    let elf_size = elf
        .program_headers
        .iter()
        .filter_map(|program_header| {
            if let Ok(type_) = program_header.get_type()
                && type_ == program::Type::Load
            {
                let ph_max_addr = program_header.virtual_addr + program_header.mem_size;
                Some(ph_max_addr as usize)
            } else {
                None
            }
        })
        .max()
        .ok_or(Error::with_message(
            Errno::ENOEXEC,
            "executable file does not has loadable sections",
        ))?;
    let map_size = elf_size.align_up(PAGE_SIZE);
    let vmar_map_options = root_vmar
        .new_map(map_size, VmPerms::empty())?
        .handle_page_faults_around();
    vmar_map_options.build()
}

/// Creates and map the corresponding segment VMO to `root_vmar`.
/// If needed, create additional anonymous mapping to represents .bss segment.
fn map_segment_vmo(
    program_header: &ProgramHeader64,
    elf_file: &Dentry,
    root_vmar: &Vmar<Full>,
    base_addr: Vaddr,
) -> Result<()> {
    trace!(
        "mem range = 0x{:x} - 0x{:x}, mem_size = 0x{:x}",
        program_header.virtual_addr,
        program_header.virtual_addr + program_header.mem_size,
        program_header.mem_size
    );
    trace!(
        "file range = 0x{:x} - 0x{:x}, file_size = 0x{:x}",
        program_header.offset,
        program_header.offset + program_header.file_size,
        program_header.file_size
    );

    let file_offset = program_header.offset as usize;
    let virtual_addr = program_header.virtual_addr as usize;
    debug_assert!(file_offset % PAGE_SIZE == virtual_addr % PAGE_SIZE);
    let segment_vmo = {
        let inode = elf_file.inode();
        inode
            .page_cache()
            .ok_or(Error::with_message(
                Errno::ENOENT,
                "executable has no page cache",
            ))?
            .to_dyn()
            .dup_independent()?
    };

    let total_map_size = {
        let vmap_start = virtual_addr.align_down(PAGE_SIZE);
        let vmap_end = (virtual_addr + program_header.mem_size as usize).align_up(PAGE_SIZE);
        vmap_end - vmap_start
    };

    let (segment_offset, segment_size) = {
        let start = file_offset.align_down(PAGE_SIZE);
        let end = (file_offset + program_header.file_size as usize).align_up(PAGE_SIZE);
        debug_assert!(total_map_size >= (program_header.file_size as usize).align_up(PAGE_SIZE));
        (start, end - start)
    };

    // Write zero as paddings. There are head padding and tail padding.
    // Head padding: if the segment's virtual address is not page-aligned,
    // then the bytes in first page from start to virtual address should be padded zeros.
    // Tail padding: If the segment's mem_size is larger than file size,
    // then the bytes that are not backed up by file content should be zeros.(usually .data/.bss sections).

    // Head padding.
    let page_offset = file_offset % PAGE_SIZE;
    if page_offset != 0 {
        let new_frame = {
            let head_frame = segment_vmo.commit_page(segment_offset)?;
            let new_frame = duplicate_frame(&head_frame)?;

            let buffer = vec![0u8; page_offset];
            new_frame.write_bytes(0, &buffer).unwrap();
            new_frame
        };
        let head_idx = segment_offset / PAGE_SIZE;
        segment_vmo.replace(new_frame, head_idx)?;
    }

    // Tail padding.
    let tail_padding_offset = program_header.file_size as usize + page_offset;
    if segment_size > tail_padding_offset {
        let new_frame = {
            let tail_frame = segment_vmo.commit_page(segment_offset + tail_padding_offset)?;
            let new_frame = duplicate_frame(&tail_frame)?;

            let buffer = vec![0u8; (segment_size - tail_padding_offset) % PAGE_SIZE];
            new_frame
                .write_bytes(tail_padding_offset % PAGE_SIZE, &buffer)
                .unwrap();
            new_frame
        };

        let tail_idx = (segment_offset + tail_padding_offset) / PAGE_SIZE;
        segment_vmo.replace(new_frame, tail_idx).unwrap();
    }

    let perms = parse_segment_perm(program_header.flags);
    let mut vm_map_options = root_vmar
        .new_map(segment_size, perms)?
        .vmo(segment_vmo)
        .vmo_offset(segment_offset)
        .vmo_limit(segment_offset + segment_size)
        .can_overwrite(true);
    let offset = base_addr + (program_header.virtual_addr as Vaddr).align_down(PAGE_SIZE);
    vm_map_options = vm_map_options.offset(offset).handle_page_faults_around();
    let map_addr = vm_map_options.build()?;

    let anonymous_map_size: usize = total_map_size.saturating_sub(segment_size);

    if anonymous_map_size > 0 {
        let mut anonymous_map_options = root_vmar
            .new_map(anonymous_map_size, perms)?
            .can_overwrite(true);
        anonymous_map_options = anonymous_map_options.offset(offset + segment_size);
        anonymous_map_options.build()?;
    }
    Ok(())
}

fn parse_segment_perm(flags: xmas_elf::program::Flags) -> VmPerms {
    let mut vm_perm = VmPerms::empty();
    if flags.is_read() {
        vm_perm |= VmPerms::READ;
    }
    if flags.is_write() {
        vm_perm |= VmPerms::WRITE;
    }
    if flags.is_execute() {
        vm_perm |= VmPerms::EXEC;
    }
    vm_perm
}

fn check_segment_align(program_header: &ProgramHeader64) -> Result<()> {
    let align = program_header.align;
    if align == 0 || align == 1 {
        // no align requirement
        return Ok(());
    }
    if !align.is_power_of_two() {
        return_errno_with_message!(Errno::ENOEXEC, "segment align is invalid.");
    }
    if program_header.offset % align != program_header.virtual_addr % align {
        return_errno_with_message!(Errno::ENOEXEC, "segment align is not satisfied.");
    }
    Ok(())
}

pub fn init_aux_vec(elf: &Elf, elf_map_addr: Vaddr, ldso_base: Option<Vaddr>) -> Result<AuxVec> {
    let mut aux_vec = AuxVec::new();
    aux_vec.set(AuxKey::AT_PAGESZ, PAGE_SIZE as _)?;
    let ph_addr = if elf.is_shared_object() {
        elf.ph_addr()? + elf_map_addr
    } else {
        elf.ph_addr()?
    };
    aux_vec.set(AuxKey::AT_PHDR, ph_addr as u64)?;
    aux_vec.set(AuxKey::AT_PHNUM, elf.ph_count() as u64)?;
    aux_vec.set(AuxKey::AT_PHENT, elf.ph_ent() as u64)?;
    let elf_entry = if elf.is_shared_object() {
        let base_load_offset = elf.base_load_address_offset();
        elf.entry_point() + elf_map_addr - base_load_offset as usize
    } else {
        elf.entry_point()
    };
    aux_vec.set(AuxKey::AT_ENTRY, elf_entry as u64)?;

    if let Some(ldso_base) = ldso_base {
        aux_vec.set(AuxKey::AT_BASE, ldso_base as u64)?;
    }
    Ok(aux_vec)
}

/// Maps the VDSO VMO to the corresponding virtual memory address.
fn map_vdso_to_vm(process_vm: &ProcessVm) -> Option<Vaddr> {
    let root_vmar = process_vm.root_vmar();
    let vdso_vmo = vdso_vmo()?;

    let options = root_vmar
        .new_map(VDSO_VMO_SIZE, VmPerms::empty())
        .unwrap()
        .vmo(vdso_vmo.dup().unwrap());

    let vdso_data_base = options.build().unwrap();
    let vdso_text_base = vdso_data_base + 0x4000;

    let data_perms = VmPerms::READ | VmPerms::WRITE;
    let text_perms = VmPerms::READ | VmPerms::EXEC;
    root_vmar
        .protect(data_perms, vdso_data_base..vdso_data_base + PAGE_SIZE)
        .unwrap();
    root_vmar
        .protect(text_perms, vdso_text_base..vdso_text_base + PAGE_SIZE)
        .unwrap();
    Some(vdso_text_base)
}
