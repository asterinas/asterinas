//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use crate::fs::file_handle::FileHandle;
use crate::process::program_loader::elf::init_stack::{init_aux_vec, InitStack};
use crate::vm::perms::VmPerms;
use crate::vm::vmo::VmoRightsOp;
use crate::{
    prelude::*,
    rights::Full,
    vm::{
        vmar::Vmar,
        vmo::{Pager, Vmo, VmoOptions},
    },
};
use align_ext::AlignExt;
use jinux_frame::vm::VmPerm;
use xmas_elf::program::{self, ProgramHeader64};

use super::elf_file::Elf;
use super::elf_segment_pager::ElfSegmentPager;

/// load elf to the root vmar. this function will  
/// 1. read the vaddr of each segment to get all elf pages.  
/// 2. create a vmo for each elf segment, create a backup pager for each segment. Then map the vmo to the root vmar.
/// 3. write proper content to the init stack.
pub fn load_elf_to_root_vmar(
    root_vmar: &Vmar<Full>,
    file_header: &[u8],
    elf_file: Arc<FileHandle>,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<ElfLoadInfo> {
    let elf = Elf::parse_elf(file_header)?;
    let map_addr = map_segment_vmos(&elf, root_vmar, elf_file)?;
    let mut aux_vec = init_aux_vec(&elf, map_addr)?;
    let mut init_stack = InitStack::new_default_config(argv, envp);
    init_stack.init(root_vmar, &elf, &mut aux_vec)?;
    let entry_point = if elf.is_shared_object() {
        elf.entry_point() + map_addr.unwrap()
    } else {
        elf.entry_point()
    };
    let elf_load_info = ElfLoadInfo::new(entry_point, init_stack.user_stack_top());
    debug!("load elf succeeds.");
    Ok(elf_load_info)
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
pub fn map_segment_vmos(
    elf: &Elf,
    root_vmar: &Vmar<Full>,
    elf_file: Arc<FileHandle>,
) -> Result<Option<Vaddr>> {
    let is_shared_object = elf.is_shared_object();
    let mut file_map_addr = None;
    for program_header in &elf.program_headers {
        let type_ = program_header
            .get_type()
            .map_err(|_| Error::with_message(Errno::ENOEXEC, "parse program header type fails"))?;
        if type_ == program::Type::Load {
            let vmo = init_segment_vmo(program_header, elf_file.clone())?;
            map_segment_vmo(
                program_header,
                vmo,
                root_vmar,
                elf_file.clone(),
                &mut file_map_addr,
                is_shared_object,
            )?;
        }
    }
    Ok(file_map_addr)
}

/// map the segment vmo to root_vmar
fn map_segment_vmo(
    program_header: &ProgramHeader64,
    vmo: Vmo,
    root_vmar: &Vmar<Full>,
    elf_file: Arc<FileHandle>,
    file_map_addr: &mut Option<Vaddr>,
    is_shared_object: bool,
) -> Result<()> {
    let perms = VmPerms::from(parse_segment_perm(program_header.flags)?);
    let offset = (program_header.virtual_addr as Vaddr).align_down(PAGE_SIZE);
    debug!(
        "map segment vmo: offset = 0x{:x}, virtual_addr = 0x{:x}",
        offset, program_header.virtual_addr
    );
    let mut vm_map_options = root_vmar.new_map(vmo, perms)?;
    // offset = 0 means the vmo can be put at any address
    if is_shared_object {
        if let Some(file_init_addr) = *file_map_addr {
            let offset = file_init_addr + offset;
            vm_map_options = vm_map_options.offset(offset);
        }
    } else {
        vm_map_options = vm_map_options.offset(offset);
    }
    let map_addr = vm_map_options.build()?;
    if is_shared_object && *file_map_addr == None {
        *file_map_addr = Some(map_addr);
    }
    Ok(())
}

/// create vmo for each segment
fn init_segment_vmo(program_header: &ProgramHeader64, elf_file: Arc<FileHandle>) -> Result<Vmo> {
    let vmo_start = (program_header.virtual_addr as Vaddr).align_down(PAGE_SIZE);
    let vmo_end = (program_header.virtual_addr as Vaddr + program_header.mem_size as Vaddr)
        .align_up(PAGE_SIZE);
    let segment_len = vmo_end - vmo_start;
    let pager = Arc::new(ElfSegmentPager::new(elf_file, &program_header)) as Arc<dyn Pager>;
    let vmo_alloc_options: VmoOptions<Full> = VmoOptions::new(segment_len).pager(pager);
    Ok(vmo_alloc_options.alloc()?.to_dyn())
}

fn parse_segment_perm(flags: xmas_elf::program::Flags) -> Result<VmPerm> {
    if !flags.is_read() {
        return_errno_with_message!(Errno::ENOEXEC, "unreadable segment");
    }
    let mut vm_perm = VmPerm::R;
    if flags.is_write() {
        vm_perm |= VmPerm::W;
    }
    if flags.is_execute() {
        vm_perm |= VmPerm::X;
    }
    Ok(vm_perm)
}
