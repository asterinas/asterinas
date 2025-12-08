// SPDX-License-Identifier: MPL-2.0

//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use core::ops::Range;

use align_ext::AlignExt;
use xmas_elf::program::{self, ProgramHeader64};

use super::elf_file::ElfHeaders;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        path::Path,
        utils::Inode,
    },
    prelude::*,
    process::process_vm::{AuxKey, AuxVec},
    vm::{perms::VmPerms, vmar::Vmar},
};

/// Loads elf to the process VMAR.
///
/// This function will map elf segments and
/// initialize process init stack.
pub fn load_elf_to_vmar(
    vmar: &Vmar,
    elf_inode: &Arc<dyn Inode>,
    fs_resolver: &FsResolver,
    elf_headers: ElfHeaders,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<ElfLoadInfo> {
    let ldso = lookup_and_parse_ldso(&elf_headers, elf_inode, fs_resolver)?;

    #[cfg_attr(
        not(any(target_arch = "x86_64", target_arch = "riscv64")),
        expect(unused_mut)
    )]
    let (_range, entry_point, mut aux_vec) =
        init_and_map_vmos(vmar, ldso, &elf_headers, elf_inode)?;

    // Map the vDSO and set the entry.
    // Since the vDSO does not require being mapped to any specific address,
    // the vDSO is mapped after the ELF file, heap, and stack.
    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    if let Some(vdso_text_base) = map_vdso_to_vmar(vmar) {
        #[cfg(target_arch = "riscv64")]
        vmar.process_vm().set_vdso_base(vdso_text_base);
        aux_vec
            .set(AuxKey::AT_SYSINFO_EHDR, vdso_text_base as u64)
            .unwrap();
    }

    vmar.process_vm()
        .map_and_write_init_stack(vmar, argv, envp, aux_vec)?;

    let user_stack_top = vmar.process_vm().init_stack().user_stack_top();
    Ok(ElfLoadInfo {
        entry_point,
        user_stack_top,
        _private: (),
    })
}

fn lookup_and_parse_ldso(
    headers: &ElfHeaders,
    elf_inode: &Arc<dyn Inode>,
    fs_resolver: &FsResolver,
) -> Result<Option<(Path, ElfHeaders)>> {
    let ldso_file = {
        let Some(ldso_path) = headers.read_ldso_path(elf_inode)? else {
            return Ok(None);
        };
        // Our FS requires the path to be valid UTF-8. This may be too restrictive.
        let ldso_path = ldso_path.into_string().map_err(|_| {
            Error::with_message(
                Errno::ENOEXEC,
                "The interpreter path specified in ELF is not a valid UTF-8 string",
            )
        })?;
        let fs_path = FsPath::try_from(ldso_path.as_str())?;
        fs_resolver.lookup(&fs_path)?
    };
    let ldso_elf = {
        let mut buf = Box::new([0u8; PAGE_SIZE]);
        let inode = ldso_file.inode();
        inode.read_bytes_at(0, &mut *buf)?;
        ElfHeaders::parse_elf(&*buf)?
    };
    Ok(Some((ldso_file, ldso_elf)))
}

fn load_ldso(vmar: &Vmar, ldso_file: &Path, ldso_elf: &ElfHeaders) -> Result<LdsoLoadInfo> {
    let range = map_segment_vmos(ldso_elf, vmar, ldso_file.inode())?;
    Ok(LdsoLoadInfo {
        entry_point: range
            .relocated_addr_of(ldso_elf.entry_point())
            .ok_or(Error::with_message(
                Errno::ENOEXEC,
                "The entry point is not in the mapped range",
            ))?,
        range,
        _private: (),
    })
}

/// Initializes the VM space and maps the VMO to the corresponding virtual memory address.
///
/// Returns the mapped range, the entry point and the auxiliary vector.
fn init_and_map_vmos(
    vmar: &Vmar,
    ldso: Option<(Path, ElfHeaders)>,
    parsed_elf: &ElfHeaders,
    elf_inode: &Arc<dyn Inode>,
) -> Result<(RelocatedRange, Vaddr, AuxVec)> {
    // After we clear process vm, if any error happens, we must call exit_group instead of return to user space.
    let ldso_load_info = if let Some((ldso_file, ldso_elf)) = ldso {
        Some(load_ldso(vmar, &ldso_file, &ldso_elf)?)
    } else {
        None
    };

    let elf_map_range = map_segment_vmos(parsed_elf, vmar, elf_inode)?;

    let mut aux_vec = {
        let ldso_base = ldso_load_info
            .as_ref()
            .map(|load_info| load_info.range.relocated_start);
        init_aux_vec(parsed_elf, elf_map_range.relocated_start, ldso_base)?
    };

    // Set AT_SECURE based on setuid/setgid bits of the executable file.
    let mode = elf_inode.mode()?;
    let secure = if mode.has_set_uid() || mode.has_set_gid() {
        1
    } else {
        0
    };
    aux_vec.set(AuxKey::AT_SECURE, secure)?;

    let entry_point = if let Some(ldso_load_info) = ldso_load_info {
        // Normal shared object
        ldso_load_info.entry_point
    } else {
        elf_map_range
            .relocated_addr_of(parsed_elf.entry_point())
            .ok_or(Error::with_message(
                Errno::ENOEXEC,
                "The entry point is not in the mapped range",
            ))?
    };

    Ok((elf_map_range, entry_point, aux_vec))
}

pub struct LdsoLoadInfo {
    /// Relocated entry point.
    pub entry_point: Vaddr,
    /// The range covering all the mapped segments.
    ///
    /// May not be page-aligned.
    pub range: RelocatedRange,
    _private: (),
}

pub struct ElfLoadInfo {
    /// Relocated entry point.
    pub entry_point: Vaddr,
    /// Address of the user stack top.
    pub user_stack_top: Vaddr,
    _private: (),
}

/// Initializes a [`Vmo`] for each segment and then map to the [`Vmar`].
///
/// This function will return the mapped range that covers all segments. The
/// range will be tight, i.e., will not include any padding bytes. So the
/// boundaries may not be page-aligned.
///
/// [`Vmo`]: crate::vm::vmo::Vmo
pub fn map_segment_vmos(
    elf: &ElfHeaders,
    vmar: &Vmar,
    elf_inode: &Arc<dyn Inode>,
) -> Result<RelocatedRange> {
    let elf_va_range = get_range_for_all_segments(elf)?;

    let map_range = if elf.is_shared_object() {
        // Relocatable object.

        // Allocate a continuous range of virtual memory for all segments in advance.
        //
        // All segments in the ELF program must be mapped to a continuous VM range to
        // ensure the relative offset of each segment not changed.
        let elf_va_range_aligned =
            elf_va_range.start.align_down(PAGE_SIZE)..elf_va_range.end.align_up(PAGE_SIZE);
        let map_size = elf_va_range_aligned.len();

        let vmar_map_options = vmar
            .new_map(map_size, VmPerms::empty())?
            .handle_page_faults_around();
        let aligned_range = vmar_map_options.build().map(|addr| addr..addr + map_size)?;

        let start_in_page_offset = elf_va_range.start - elf_va_range_aligned.start;
        let end_in_page_offset = elf_va_range_aligned.end - elf_va_range.end;

        aligned_range.start + start_in_page_offset..aligned_range.end - end_in_page_offset
    } else {
        // Not relocatable object. Map as-is.
        elf_va_range.clone()
    };

    let relocated_range =
        RelocatedRange::new(elf_va_range, map_range.start).expect("Mapped range overflows");

    for program_header in &elf.program_headers {
        let type_ = program_header.get_type().map_err(|_| {
            Error::with_message(Errno::ENOEXEC, "Failed to parse the program header")
        })?;
        if type_ == program::Type::Load {
            check_segment_align(program_header)?;

            let map_at = relocated_range
                .relocated_addr_of(program_header.virtual_addr as Vaddr)
                .expect("Address not covered by `get_range_for_all_segments`");

            map_segment_vmo(program_header, elf_inode, vmar, map_at)?;
        }
    }

    Ok(relocated_range)
}

/// A virtual range and its relocated address.
pub struct RelocatedRange {
    original_range: Range<Vaddr>,
    relocated_start: Vaddr,
}

impl RelocatedRange {
    /// Creates a new `RelocatedRange`.
    ///
    /// If the relocated address overflows, it will return `None`.
    pub fn new(original_range: Range<Vaddr>, relocated_start: Vaddr) -> Option<Self> {
        relocated_start.checked_add(original_range.len())?;
        Some(Self {
            original_range,
            relocated_start,
        })
    }

    /// Gets the relocated address of an address in the original range.
    ///
    /// If the provided address is not in the original range, it will return `None`.
    pub fn relocated_addr_of(&self, addr: Vaddr) -> Option<Vaddr> {
        if self.original_range.contains(&addr) {
            Some(addr - self.original_range.start + self.relocated_start)
        } else {
            None
        }
    }
}

/// Returns the range that covers all segments in the ELF file.
///
/// The range must be tight, i.e., will not include any padding bytes. So the
/// boundaries may not be page-aligned.
fn get_range_for_all_segments(elf: &ElfHeaders) -> Result<Range<Vaddr>> {
    let loadable_ranges_iter = elf.program_headers.iter().filter_map(|ph| {
        if let Ok(program::Type::Load) = ph.get_type() {
            Some((ph.virtual_addr as Vaddr)..((ph.virtual_addr + ph.mem_size) as Vaddr))
        } else {
            None
        }
    });

    let min_addr =
        loadable_ranges_iter
            .clone()
            .map(|r| r.start)
            .min()
            .ok_or(Error::with_message(
                Errno::ENOEXEC,
                "Executable file does not has loadable sections",
            ))?;

    let max_addr = loadable_ranges_iter
        .map(|r| r.end)
        .max()
        .expect("The range set contains minimum but no maximum");

    Ok(min_addr..max_addr)
}

/// Creates and map the corresponding segment VMO to `vmar`.
/// If needed, create additional anonymous mapping to represents .bss segment.
fn map_segment_vmo(
    program_header: &ProgramHeader64,
    elf_inode: &Arc<dyn Inode>,
    vmar: &Vmar,
    map_at: Vaddr,
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
        elf_inode.page_cache().ok_or(Error::with_message(
            Errno::ENOENT,
            "executable has no page cache",
        ))?
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

    let perms = parse_segment_perm(program_header.flags);
    let offset = map_at.align_down(PAGE_SIZE);
    if segment_size != 0 {
        let mut vm_map_options = vmar
            .new_map(segment_size, perms)?
            .vmo(segment_vmo.clone())
            .vmo_offset(segment_offset)
            .can_overwrite(true);
        vm_map_options = vm_map_options.offset(offset).handle_page_faults_around();
        let map_addr = vm_map_options.build()?;

        // Write zero as paddings if the tail is not page-aligned and map size
        // is larger than file size (e.g., `.bss`). The mapping is by default
        // private so the writes will trigger copy-on-write. Ignore errors if
        // the permissions do not allow writing.
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/binfmt_elf.c#L410-L422>
        if program_header.file_size < program_header.mem_size {
            let tail_start_vaddr =
                map_addr + virtual_addr % PAGE_SIZE + program_header.file_size as usize;
            if tail_start_vaddr < map_addr + segment_size {
                let zero_size = PAGE_SIZE - tail_start_vaddr % PAGE_SIZE;
                let res = vmar.fill_zeros_remote(tail_start_vaddr, zero_size);
                if let Err((e, _)) = res
                    && perms.contains(VmPerms::WRITE)
                {
                    return Err(e);
                }
            };
        }
    }

    let anonymous_map_size: usize = total_map_size.saturating_sub(segment_size);
    if anonymous_map_size > 0 {
        let mut anonymous_map_options =
            vmar.new_map(anonymous_map_size, perms)?.can_overwrite(true);
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

pub fn init_aux_vec(
    elf: &ElfHeaders,
    elf_map_addr: Vaddr,
    ldso_base: Option<Vaddr>,
) -> Result<AuxVec> {
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

/// Maps the vDSO VMO to the corresponding virtual memory address.
#[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
fn map_vdso_to_vmar(vmar: &Vmar) -> Option<Vaddr> {
    use crate::vdso::{VDSO_VMO_LAYOUT, vdso_vmo};

    let vdso_vmo = vdso_vmo()?;

    let options = vmar
        .new_map(VDSO_VMO_LAYOUT.size, VmPerms::empty())
        .unwrap()
        .vmo(vdso_vmo);

    let vdso_vmo_base = options.build().unwrap();
    let vdso_data_base = vdso_vmo_base + VDSO_VMO_LAYOUT.data_segment_offset;
    let vdso_text_base = vdso_vmo_base + VDSO_VMO_LAYOUT.text_segment_offset;

    let data_perms = VmPerms::READ;
    let text_perms = VmPerms::READ | VmPerms::EXEC;
    vmar.protect(
        data_perms,
        vdso_data_base..(vdso_data_base + VDSO_VMO_LAYOUT.data_segment_size),
    )
    .unwrap();
    vmar.protect(
        text_perms,
        vdso_text_base..(vdso_text_base + VDSO_VMO_LAYOUT.text_segment_size),
    )
    .unwrap();
    Some(vdso_text_base)
}
