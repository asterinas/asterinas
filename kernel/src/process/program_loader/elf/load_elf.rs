// SPDX-License-Identifier: MPL-2.0

//! This module is used to parse elf file content to get elf_load_info.
//! When create a process from elf file, we will use the elf_load_info to construct the VmSpace

use core::ops::Range;

use align_ext::AlignExt;
use aster_rights::Full;
use ostd::{
    mm::{CachePolicy, PageFlags, PageProperty, VmIo},
    task::disable_preempt,
};
use xmas_elf::program::{self, ProgramHeader64};

use super::elf_file::ElfHeaders;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver, AT_FDCWD},
        path::Path,
    },
    prelude::*,
    process::process_vm::{AuxKey, AuxVec, ProcessVm},
    vm::{
        perms::VmPerms,
        util::duplicate_frame,
        vmar::Vmar,
        vmo::{CommitFlags, VmoRightsOp},
    },
};

/// Loads elf to the process vm.
///
/// This function will map elf segments and
/// initialize process init stack.
pub fn load_elf_to_vm(
    process_vm: &ProcessVm,
    elf_file: Path,
    fs_resolver: &FsResolver,
    elf_headers: ElfHeaders,
    argv: Vec<CString>,
    envp: Vec<CString>,
) -> Result<ElfLoadInfo> {
    let ldso = lookup_and_parse_ldso(&elf_headers, &elf_file, fs_resolver)?;

    match init_and_map_vmos(process_vm, ldso, &elf_headers, &elf_file) {
        #[cfg_attr(
            not(any(target_arch = "x86_64", target_arch = "riscv64")),
            expect(unused_mut)
        )]
        Ok((_range, entry_point, mut aux_vec)) => {
            // Map the vDSO and set the entry.
            // Since the vDSO does not require being mapped to any specific address,
            // the vDSO is mapped after the ELF file, heap, and stack.
            #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
            if let Some(vdso_text_base) = map_vdso_to_vm(process_vm) {
                #[cfg(target_arch = "riscv64")]
                process_vm.set_vdso_base(vdso_text_base);
                aux_vec
                    .set(AuxKey::AT_SYSINFO_EHDR, vdso_text_base as u64)
                    .unwrap();
            }

            process_vm.map_and_write_init_stack(argv, envp, aux_vec)?;

            let user_stack_top = process_vm.user_stack_top();
            Ok(ElfLoadInfo {
                entry_point,
                user_stack_top,
                _private: (),
            })
        }
        Err(err) => {
            // Since the process_vm is in invalid state,
            // the process cannot return to user space again,
            // so `Vmar::clear` and `do_exit_group` are called here.
            // FIXME: sending a fault signal is an alternative approach.
            process_vm.lock_root_vmar().unwrap().clear().unwrap();

            // The process will exit and the error code will be ignored.
            Err(err)
        }
    }
}

fn lookup_and_parse_ldso(
    headers: &ElfHeaders,
    elf_file: &Path,
    fs_resolver: &FsResolver,
) -> Result<Option<(Path, ElfHeaders)>> {
    let ldso_file = {
        let Some(ldso_path) = headers.read_ldso_path(elf_file)? else {
            return Ok(None);
        };
        // Our FS requires the path to be valid UTF-8. This may be too restrictive.
        let ldso_path = ldso_path.into_string().map_err(|_| {
            Error::with_message(
                Errno::ENOEXEC,
                "The interpreter path specified in ELF is not a valid UTF-8 string",
            )
        })?;
        let fs_path = FsPath::new(AT_FDCWD, ldso_path.as_str())?;
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

fn load_ldso(
    root_vmar: &Vmar<Full>,
    ldso_file: &Path,
    ldso_elf: &ElfHeaders,
) -> Result<LdsoLoadInfo> {
    let range = map_segment_vmos(ldso_elf, root_vmar, ldso_file)?;
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
    process_vm: &ProcessVm,
    ldso: Option<(Path, ElfHeaders)>,
    parsed_elf: &ElfHeaders,
    elf_file: &Path,
) -> Result<(RelocatedRange, Vaddr, AuxVec)> {
    let process_vmar = process_vm.lock_root_vmar();
    let root_vmar = process_vmar.unwrap();

    // After we clear process vm, if any error happens, we must call exit_group instead of return to user space.
    let ldso_load_info = if let Some((ldso_file, ldso_elf)) = ldso {
        Some(load_ldso(root_vmar, &ldso_file, &ldso_elf)?)
    } else {
        None
    };

    let elf_map_range = map_segment_vmos(parsed_elf, root_vmar, elf_file)?;

    let aux_vec = {
        let ldso_base = ldso_load_info
            .as_ref()
            .map(|load_info| load_info.range.relocated_start);
        init_aux_vec(parsed_elf, elf_map_range.relocated_start, ldso_base)?
    };

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

/// Initializes a [`Vmo`] for each segment and then map to the root [`Vmar`].
///
/// This function will return the mapped range that covers all segments. The
/// range will be tight, i.e., will not include any padding bytes. So the
/// boundaries may not be page-aligned.
///
/// [`Vmo`]: crate::vm::vmo::Vmo
pub fn map_segment_vmos(
    elf: &ElfHeaders,
    root_vmar: &Vmar<Full>,
    elf_file: &Path,
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

        let vmar_map_options = root_vmar
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

            map_segment_vmo(program_header, elf_file, root_vmar, map_at)?;
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

/// Creates and map the corresponding segment VMO to `root_vmar`.
/// If needed, create additional anonymous mapping to represents .bss segment.
fn map_segment_vmo(
    program_header: &ProgramHeader64,
    elf_file: &Path,
    root_vmar: &Vmar<Full>,
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
        let inode = elf_file.inode();
        inode
            .page_cache()
            .ok_or(Error::with_message(
                Errno::ENOENT,
                "executable has no page cache",
            ))?
            .to_dyn()
            .dup()?
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
        let mut vm_map_options = root_vmar
            .new_map(segment_size, perms)?
            .vmo(segment_vmo.dup()?)
            .vmo_offset(segment_offset)
            .can_overwrite(true);
        vm_map_options = vm_map_options.offset(offset).handle_page_faults_around();
        let map_addr = vm_map_options.build()?;

        // Write zero as paddings. There are head padding and tail padding.
        // Head padding: if the segment's virtual address is not page-aligned,
        // then the bytes in first page from start to virtual address should be padded zeros.
        // Tail padding: If the segment's mem_size is larger than file size,
        // then the bytes that are not backed up by file content should be zeros.(usually .data/.bss sections).

        // Head padding.
        let page_offset = file_offset % PAGE_SIZE;
        let head_frame = if page_offset != 0 {
            let head_frame =
                segment_vmo.commit_on(segment_offset / PAGE_SIZE, CommitFlags::empty())?;
            let new_frame = duplicate_frame(&head_frame)?;

            let buffer = vec![0u8; page_offset];
            new_frame.write_bytes(0, &buffer).unwrap();
            Some(new_frame)
        } else {
            None
        };

        // Tail padding.
        let tail_padding_offset = program_header.file_size as usize + page_offset;
        let tail_frame_and_addr = if segment_size > tail_padding_offset {
            let tail_frame = {
                let offset_index = (segment_offset + tail_padding_offset) / PAGE_SIZE;
                segment_vmo.commit_on(offset_index, CommitFlags::empty())?
            };
            let new_frame = duplicate_frame(&tail_frame)?;

            let buffer = vec![0u8; (segment_size - tail_padding_offset) % PAGE_SIZE];
            new_frame
                .write_bytes(tail_padding_offset % PAGE_SIZE, &buffer)
                .unwrap();

            let tail_page_addr = map_addr + tail_padding_offset.align_down(PAGE_SIZE);
            Some((new_frame, tail_page_addr))
        } else {
            None
        };

        let preempt_guard = disable_preempt();
        let mut cursor = root_vmar
            .vm_space()
            .cursor_mut(&preempt_guard, &(map_addr..map_addr + segment_size))?;
        let page_flags = PageFlags::from(perms) | PageFlags::ACCESSED;

        if let Some(head_frame) = head_frame {
            cursor.map(
                head_frame.into(),
                PageProperty::new_user(page_flags, CachePolicy::Writeback),
            );
        }

        if let Some((tail_frame, tail_page_addr)) = tail_frame_and_addr {
            cursor.jump(tail_page_addr)?;
            cursor.map(
                tail_frame.into(),
                PageProperty::new_user(page_flags, CachePolicy::Writeback),
            );
        }
    }

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
fn map_vdso_to_vm(process_vm: &ProcessVm) -> Option<Vaddr> {
    use crate::vdso::{vdso_vmo, VDSO_VMO_LAYOUT};

    let process_vmar = process_vm.lock_root_vmar();
    let root_vmar = process_vmar.unwrap();
    let vdso_vmo = vdso_vmo()?;

    let options = root_vmar
        .new_map(VDSO_VMO_LAYOUT.size, VmPerms::empty())
        .unwrap()
        .vmo(vdso_vmo.dup().unwrap());

    let vdso_vmo_base = options.build().unwrap();
    let vdso_data_base = vdso_vmo_base + VDSO_VMO_LAYOUT.data_segment_offset;
    let vdso_text_base = vdso_vmo_base + VDSO_VMO_LAYOUT.text_segment_offset;

    let data_perms = VmPerms::READ;
    let text_perms = VmPerms::READ | VmPerms::EXEC;
    root_vmar
        .protect(
            data_perms,
            vdso_data_base..(vdso_data_base + VDSO_VMO_LAYOUT.data_segment_size),
        )
        .unwrap();
    root_vmar
        .protect(
            text_perms,
            vdso_text_base..(vdso_text_base + VDSO_VMO_LAYOUT.text_segment_size),
        )
        .unwrap();
    Some(vdso_text_base)
}
