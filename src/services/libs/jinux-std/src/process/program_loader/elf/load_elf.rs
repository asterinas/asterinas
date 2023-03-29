//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use crate::fs::file_handle::FileHandle;
use crate::fs::fs_resolver::{FsPath, FsResolver, AT_FDCWD};
use crate::fs::utils::AccessMode;
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
    let map_addr = map_segment_vmos(&elf, root_vmar, elf_file)?;
    let mut aux_vec = init_aux_vec(&elf, map_addr)?;
    let mut init_stack = InitStack::new_default_config(argv, envp);
    init_stack.init(root_vmar, &elf, &ldso_load_info, &mut aux_vec)?;
    let entry_point = if let Some(ldso_load_info) = ldso_load_info {
        // Normal shared object
        ldso_load_info.entry_point()
    } else {
        if elf.is_shared_object() {
            // ldso itself
            elf.entry_point() + map_addr.unwrap()
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
    if let Ok(ldso_path) = elf.ldso_path(file_header) && elf.is_shared_object(){
        trace!("ldso_path = {:?}", ldso_path);
        let fs_path = FsPath::new(AT_FDCWD, &ldso_path)?;
        let ldso_file = fs_resolver.open(&fs_path, AccessMode::O_RDONLY as u32, 0)?;
        let mut buf = Box::new([0u8; PAGE_SIZE]);
        let ldso_header = ldso_file.read(&mut *buf)?;
        let ldso_elf = Elf::parse_elf(&*buf)?;
        let ldso_file = Arc::new(FileHandle::new_inode_handle(ldso_file));
        let map_addr = map_segment_vmos(&ldso_elf, root_vmar, ldso_file)?.unwrap();
        return Ok(LdsoLoadInfo::new(ldso_elf.entry_point() + map_addr, map_addr));
    }
    // There are three reasons that an executable may lack ldso_path,
    // 1. this is a statically linked executable,
    // 2. the shared object is invalid,
    // 3. the shared object is ldso itself,
    // we don't try to distinguish these cases and just let it go.
    return_errno_with_message!(Errno::ENOEXEC, "cannot find ldso for shared object");
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
pub fn map_segment_vmos(
    elf: &Elf,
    root_vmar: &Vmar<Full>,
    elf_file: Arc<FileHandle>,
) -> Result<Option<Vaddr>> {
    // all segments of the shared object must be mapped to a continuous vm range
    // to ensure the relative offset of each segment not changed.
    let file_map_addr = if elf.is_shared_object() {
        Some(hint_elf_map_addr(elf, root_vmar)?)
    } else {
        None
    };
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
                &file_map_addr,
            )?;
        }
    }
    Ok(file_map_addr)
}

fn hint_elf_map_addr(elf: &Elf, root_vmar: &Vmar<Full>) -> Result<Vaddr> {
    let mut size = 0;
    for program_header in &elf.program_headers {
        let ph_size = program_header.virtual_addr + program_header.mem_size;
        if ph_size > size {
            size = ph_size;
        }
    }
    root_vmar.hint_map_addr(size as usize)
}

/// map the segment vmo to root_vmar
fn map_segment_vmo(
    program_header: &ProgramHeader64,
    vmo: Vmo,
    root_vmar: &Vmar<Full>,
    elf_file: Arc<FileHandle>,
    file_map_addr: &Option<Vaddr>,
) -> Result<()> {
    let perms = VmPerms::from(parse_segment_perm(program_header.flags)?);
    // let perms = VmPerms::READ | VmPerms::WRITE | VmPerms::EXEC;
    let offset = (program_header.virtual_addr as Vaddr).align_down(PAGE_SIZE);
    trace!(
        "map segment vmo: virtual addr = 0x{:x}, size = 0x{:x}, perms = {:?}",
        program_header.virtual_addr,
        program_header.file_size,
        perms
    );
    let mut vm_map_options = root_vmar.new_map(vmo, perms)?;
    if let Some(file_map_addr) = *file_map_addr {
        let offset = file_map_addr + offset;
        vm_map_options = vm_map_options.offset(offset);
    } else {
        vm_map_options = vm_map_options.offset(offset);
    }
    let map_addr = vm_map_options.build()?;
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
