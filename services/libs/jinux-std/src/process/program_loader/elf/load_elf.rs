//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use crate::fs::fs_resolver::{FsPath, FsResolver, AT_FDCWD};
use crate::fs::utils::Dentry;
use crate::process::program_loader::elf::init_stack::{init_aux_vec, InitStack};
use crate::vm::perms::VmPerms;
use crate::vm::vmo::{VmoOptions, VmoRightsOp};
use crate::{
    prelude::*,
    vm::{vmar::Vmar, vmo::Vmo},
};
use align_ext::AlignExt;
use jinux_frame::vm::{VmIo, VmPerm};
use jinux_rights::{Full, Rights};
use xmas_elf::program::{self, ProgramHeader64};

use super::elf_file::Elf;

/// load elf to the root vmar. this function will  
/// 1. read the vaddr of each segment to get all elf pages.  
/// 2. create a vmo for each elf segment, create a pager for each segment. Then map the vmo to the root vmar.
/// 3. write proper content to the init stack.
pub fn load_elf_to_root_vmar(
    root_vmar: &Vmar<Full>,
    file_header: &[u8],
    elf_file: Arc<Dentry>,
    fs_resolver: &FsResolver,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<ElfLoadInfo> {
    let elf = Elf::parse_elf(file_header)?;
    let ldso_load_info = if let Ok(ldso_load_info) =
        load_ldso_for_shared_object(root_vmar, &elf, file_header, fs_resolver)
    {
        Some(ldso_load_info)
    } else {
        None
    };
    let map_addr = map_segment_vmos(&elf, root_vmar, &elf_file)?;
    let mut aux_vec = init_aux_vec(&elf, map_addr)?;
    let mut init_stack = InitStack::new_default_config(argv, envp);
    init_stack.init(root_vmar, &elf, &ldso_load_info, &mut aux_vec)?;
    let entry_point = if let Some(ldso_load_info) = ldso_load_info {
        // Normal shared object
        ldso_load_info.entry_point()
    } else {
        if elf.is_shared_object() {
            // ldso itself
            elf.entry_point() + map_addr
        } else {
            // statically linked executable
            elf.entry_point()
        }
    };

    let elf_load_info = ElfLoadInfo::new(entry_point, init_stack.user_stack_top());
    debug!("load elf succeeds.");
    Ok(elf_load_info)
}

fn load_ldso_for_shared_object(
    root_vmar: &Vmar<Full>,
    elf: &Elf,
    file_header: &[u8],
    fs_resolver: &FsResolver,
) -> Result<LdsoLoadInfo> {
    if !elf.is_shared_object() {
        return_errno_with_message!(Errno::EINVAL, "not shared object");
    }
    let ldso_file = {
        let ldso_path = elf.ldso_path(file_header)?;
        let fs_path = FsPath::new(AT_FDCWD, &ldso_path)?;
        fs_resolver.lookup(&fs_path)?
    };
    let ldso_elf = {
        let mut buf = Box::new([0u8; PAGE_SIZE]);
        let vnode = ldso_file.vnode();
        vnode.read_at(0, &mut *buf)?;
        Elf::parse_elf(&*buf)?
    };
    let map_addr = map_segment_vmos(&ldso_elf, root_vmar, &ldso_file)?;
    Ok(LdsoLoadInfo::new(
        ldso_elf.entry_point() + map_addr,
        map_addr,
    ))
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

/// init vmo for each segment and then map segment to root vmar
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
            let vmo = init_segment_vmo(program_header, elf_file)?;
            map_segment_vmo(program_header, vmo, root_vmar, base_addr)?;
        }
    }
    Ok(base_addr)
}

fn base_map_addr(elf: &Elf, root_vmar: &Vmar<Full>) -> Result<Vaddr> {
    let elf_size = elf
        .program_headers
        .iter()
        .filter_map(|program_header| {
            if let Ok(type_) = program_header.get_type() && type_ == program::Type::Load {
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
    let vmo = VmoOptions::<Rights>::new(0).alloc()?;
    let vmar_map_options = root_vmar.new_map(vmo, VmPerms::empty())?.size(map_size);
    vmar_map_options.build()
}

/// map the segment vmo to root_vmar
fn map_segment_vmo(
    program_header: &ProgramHeader64,
    vmo: Vmo,
    root_vmar: &Vmar<Full>,
    base_addr: Vaddr,
) -> Result<()> {
    let perms = VmPerms::from(parse_segment_perm(program_header.flags));
    let offset = (program_header.virtual_addr as Vaddr).align_down(PAGE_SIZE);
    trace!(
        "map segment vmo: virtual addr = 0x{:x}, size = 0x{:x}, perms = {:?}",
        offset,
        program_header.mem_size,
        perms
    );
    let mut vm_map_options = root_vmar.new_map(vmo, perms)?.can_overwrite(true);
    let offset = base_addr + offset;
    vm_map_options = vm_map_options.offset(offset);
    let map_addr = vm_map_options.build()?;
    Ok(())
}

/// create vmo for each segment
fn init_segment_vmo(program_header: &ProgramHeader64, elf_file: &Dentry) -> Result<Vmo> {
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
    let page_cache_vmo = {
        let vnode = elf_file.vnode();
        vnode.page_cache().ok_or(Error::with_message(
            Errno::ENOENT,
            "executable has no page cache",
        ))?
    };

    let segment_vmo = {
        let vmo_offset = file_offset.align_down(PAGE_SIZE);
        let map_start = virtual_addr.align_down(PAGE_SIZE);
        let map_end = (virtual_addr + program_header.mem_size as usize).align_up(PAGE_SIZE);
        let vmo_size = map_end - map_start;
        debug_assert!(vmo_size >= (program_header.file_size as usize).align_up(PAGE_SIZE));
        page_cache_vmo
            .new_cow_child(vmo_offset..vmo_offset + vmo_size)?
            .alloc()?
    };

    // Write zero as paddings. There are head padding and tail padding.
    // Head padding: if the segment's virtual address is not page-aligned,
    // then the bytes in first page from start to virtual address should be padded zeros.
    // Tail padding: If the segment's mem_size is larger than file size,
    // then the bytes that are not backed up by file content should be zeros.(usually .data/.bss sections).
    // FIXME: Head padding may be removed.

    // Head padding.
    let page_offset = file_offset % PAGE_SIZE;
    if page_offset != 0 {
        let buffer = vec![0u8; page_offset];
        segment_vmo.write_bytes(0, &buffer)?;
    }
    // Tail padding.
    let segment_vmo_size = segment_vmo.size();
    let tail_padding_offset = program_header.file_size as usize + page_offset;
    if segment_vmo_size > tail_padding_offset {
        let buffer = vec![0u8; segment_vmo_size - tail_padding_offset];
        segment_vmo.write_bytes(tail_padding_offset, &buffer)?;
    }
    Ok(segment_vmo.to_dyn())
}

fn parse_segment_perm(flags: xmas_elf::program::Flags) -> VmPerm {
    let mut vm_perm = VmPerm::empty();
    if flags.is_read() {
        vm_perm |= VmPerm::R;
    }
    if flags.is_write() {
        vm_perm |= VmPerm::W;
    }
    if flags.is_execute() {
        vm_perm |= VmPerm::X;
    }
    vm_perm
}

fn check_segment_align(program_header: &ProgramHeader64) -> Result<()> {
    let align = program_header.align;
    if align == 0 || align == 1 {
        // no align requirement
        return Ok(());
    }
    debug_assert!(align.is_power_of_two());
    if !align.is_power_of_two() {
        return_errno_with_message!(Errno::ENOEXEC, "segment align is invalid.");
    }
    debug_assert!(program_header.offset % align == program_header.virtual_addr % align);
    if program_header.offset % align != program_header.virtual_addr % align {
        return_errno_with_message!(Errno::ENOEXEC, "segment align is not satisfied.");
    }
    Ok(())
}
