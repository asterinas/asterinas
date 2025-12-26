// SPDX-License-Identifier: MPL-2.0

//! ELF file parser.

use align_ext::AlignExt;

use super::{
    elf_file::{ElfHeaders, LoadablePhdr},
    relocate::RelocatedRange,
};
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        path::Path,
        utils::Inode,
    },
    prelude::*,
    process::{
        process_vm::{AuxKey, AuxVec},
        program_loader::check_executable_inode,
    },
    vm::{
        perms::VmPerms,
        vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, Vmar},
        vmo::Vmo,
    },
};

pub struct ElfLoadInfo {
    /// The relocated entry point.
    pub entry_point: Vaddr,
    /// The top address of the user stack.
    pub user_stack_top: Vaddr,
}

/// Loads an ELF file to the process VMAR.
///
/// This function will map ELF segments and
/// initialize the init stack and heap.
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
        map_vmos_and_build_aux_vec(vmar, ldso, &elf_headers, elf_inode)?;

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
    })
}

fn lookup_and_parse_ldso(
    headers: &ElfHeaders,
    elf_inode: &Arc<dyn Inode>,
    fs_resolver: &FsResolver,
) -> Result<Option<(Path, ElfHeaders)>> {
    let ldso_file = {
        let ldso_path = if let Some(interp_phdr) = headers.interp_phdr() {
            interp_phdr.read_ldso_path(elf_inode)?
        } else {
            return Ok(None);
        };

        // Our FS requires the path to be valid UTF-8. This may be too restrictive.
        let ldso_path = ldso_path.into_string().map_err(|_| {
            Error::with_message(
                Errno::ENOEXEC,
                "the interpreter path is not a valid UTF-8 string",
            )
        })?;

        let fs_path = FsPath::try_from(ldso_path.as_str())?;
        fs_resolver.lookup(&fs_path)?
    };

    let ldso_elf = {
        let inode = ldso_file.inode();
        check_executable_inode(inode.as_ref())?;

        let mut buf = Box::new([0u8; PAGE_SIZE]);
        let len = inode.read_bytes_at(0, &mut *buf)?;
        if len < ElfHeaders::LEN {
            return_errno_with_message!(Errno::EIO, "the interpreter format is invalid");
        }

        ElfHeaders::parse(&buf[..len])
            .map_err(|_| Error::with_message(Errno::ELIBBAD, "the interpreter format is invalid"))?
    };

    Ok(Some((ldso_file, ldso_elf)))
}

/// Maps the VMOs to the corresponding virtual memory addresses and builds the auxiliary vector.
///
/// Returns the mapped range, the entry point, and the auxiliary vector.
fn map_vmos_and_build_aux_vec(
    vmar: &Vmar,
    ldso: Option<(Path, ElfHeaders)>,
    parsed_elf: &ElfHeaders,
    elf_inode: &Arc<dyn Inode>,
) -> Result<(RelocatedRange, Vaddr, AuxVec)> {
    let ldso_load_info = if let Some((ldso_file, ldso_elf)) = ldso {
        Some(load_ldso(vmar, &ldso_file, &ldso_elf)?)
    } else {
        None
    };

    let elf_map_range = map_segment_vmos(parsed_elf, vmar, elf_inode)?;

    let mut aux_vec = {
        let ldso_base = ldso_load_info
            .as_ref()
            .map(|load_info| load_info.range.relocated_start());
        init_aux_vec(parsed_elf, &elf_map_range, ldso_base)?
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
        ldso_load_info.entry_point
    } else {
        elf_map_range
            .relocated_addr_of(parsed_elf.entry_point())
            .ok_or_else(|| {
                Error::with_message(
                    Errno::ENOEXEC,
                    "the entry point is not located in any segments",
                )
            })?
    };

    Ok((elf_map_range, entry_point, aux_vec))
}

struct LdsoLoadInfo {
    /// The relocated entry point.
    entry_point: Vaddr,
    /// The range covering all the mapped segments.
    ///
    /// Note that the range may not be page-aligned.
    range: RelocatedRange,
}

fn load_ldso(vmar: &Vmar, ldso_file: &Path, ldso_elf: &ElfHeaders) -> Result<LdsoLoadInfo> {
    let range = map_segment_vmos(ldso_elf, vmar, ldso_file.inode())?;
    let entry_point = range
        .relocated_addr_of(ldso_elf.entry_point())
        .ok_or_else(|| {
            Error::with_message(
                Errno::ENOEXEC,
                "the entry point is not located in any segments",
            )
        })?;
    Ok(LdsoLoadInfo { entry_point, range })
}

/// Initializes a [`Vmo`] for each segment and then map to the [`Vmar`].
///
/// This function will return the mapped range that covers all segments. The
/// range will be tight, i.e., will not include any padding bytes. So the
/// boundaries may not be page-aligned.
///
/// [`Vmo`]: crate::vm::vmo::Vmo
fn map_segment_vmos(
    elf: &ElfHeaders,
    vmar: &Vmar,
    elf_inode: &Arc<dyn Inode>,
) -> Result<RelocatedRange> {
    let elf_va_range = elf.calc_total_vaddr_bounds();

    let map_range = if elf.is_shared_object() {
        // Relocatable object.

        let align = elf.max_load_align();

        // Given that `elf_va_range` is guaranteed to be below `VMAR_CAP_ADDR`, as long as
        // `VMAR_CAP_ADDR * 2` does not overflow, the following `align_up(align)` cannot overflow
        // either.
        const { assert!(VMAR_CAP_ADDR.checked_mul(2).is_some()) };

        // Allocate a continuous range of virtual memory for all segments in advance.
        //
        // All segments in the ELF program must be mapped to a continuous VM range to
        // ensure the relative offset of each segment not changed.
        let elf_va_range_aligned =
            elf_va_range.start.align_down(align)..elf_va_range.end.align_up(align);
        let map_size = elf_va_range_aligned.len();

        let vmar_map_options = vmar.new_map(map_size, VmPerms::empty())?.align(align);
        let aligned_range = vmar_map_options.build().map(|addr| addr..addr + map_size)?;

        let start_offset = elf_va_range.start - elf_va_range_aligned.start;
        let end_offset = elf_va_range_aligned.end - elf_va_range.end;

        aligned_range.start + start_offset..aligned_range.end - end_offset
    } else {
        // Not relocatable object. Map as-is.

        if elf_va_range.start < VMAR_LOWEST_ADDR {
            return_errno_with_message!(Errno::EPERM, "the mapping address is too small");
        }

        // Allocate a continuous range of virtual memory for all segments in advance.
        //
        // This is to ensure that the range does not conflict with other objects, such
        // as the interpreter.
        let elf_va_range_aligned =
            elf_va_range.start.align_down(PAGE_SIZE)..elf_va_range.end.align_up(PAGE_SIZE);
        let map_size = elf_va_range_aligned.len();

        vmar.new_map(map_size, VmPerms::empty())?
            .offset(elf_va_range_aligned.start)
            .build()?;

        elf_va_range.clone()
    };

    let relocated_range = RelocatedRange::new(elf_va_range, map_range.start)
        .expect("`map_range` should not overflow");

    let Some(elf_vmo) = elf_inode.page_cache() else {
        return_errno_with_message!(Errno::ENOEXEC, "the executable has no page cache");
    };
    for loadable_phdr in elf.loadable_phdrs() {
        let map_at = relocated_range
            .relocated_addr_of(loadable_phdr.virt_range().start)
            .expect("`calc_total_vaddr_bounds()` should cover all segments");
        map_segment_vmo(loadable_phdr, &elf_vmo, vmar, map_at)?;
    }

    Ok(relocated_range)
}

/// Creates and maps the segment VMO to the VMAR.
///
/// Additional anonymous mappings will be created to represent trailing bytes, if any. For example,
/// this applies to the `.bss` segment.
fn map_segment_vmo(
    loadable_phdr: &LoadablePhdr,
    elf_vmo: &Arc<Vmo>,
    vmar: &Vmar,
    map_at: Vaddr,
) -> Result<()> {
    let virt_range = loadable_phdr.virt_range();
    let file_range = loadable_phdr.file_range();
    trace!(
        "ELF segment: virt_range = {:#x?}, file_range = {:#x?}",
        virt_range, file_range,
    );

    let total_map_size = {
        let vmap_start = virt_range.start.align_down(PAGE_SIZE);
        let vmap_end = virt_range.end.align_up(PAGE_SIZE);
        vmap_end - vmap_start
    };

    let (segment_offset, segment_size) = {
        let start = file_range.start.align_down(PAGE_SIZE);
        let end = file_range.end.align_up(PAGE_SIZE);
        (start, end - start)
    };

    let perms = loadable_phdr.vm_perms();
    let offset = map_at.align_down(PAGE_SIZE);

    if segment_size != 0 {
        let mut vm_map_options = vmar
            .new_map(segment_size, perms)?
            .vmo(elf_vmo.clone())
            .vmo_offset(segment_offset)
            .can_overwrite(true);
        vm_map_options = vm_map_options.offset(offset).handle_page_faults_around();
        let map_addr = vm_map_options.build()?;

        // Write zero as paddings if the tail is not page-aligned and map size
        // is larger than file size (e.g., `.bss`). The mapping is by default
        // private so the writes will trigger copy-on-write. Ignore errors if
        // the permissions do not allow writing.
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/binfmt_elf.c#L410-L422>
        let vaddr_to_zero = map_addr + (file_range.end - segment_offset);
        let size_to_zero = map_addr + segment_size - vaddr_to_zero;
        if size_to_zero != 0 {
            let res = vmar.fill_zeros_remote(vaddr_to_zero, size_to_zero);
            if let Err((err, _)) = res
                && perms.contains(VmPerms::WRITE)
            {
                return Err(err);
            }
        }
    }

    let anonymous_map_size = total_map_size - segment_size;
    if anonymous_map_size > 0 {
        let mut anonymous_map_options =
            vmar.new_map(anonymous_map_size, perms)?.can_overwrite(true);
        anonymous_map_options = anonymous_map_options.offset(offset + segment_size);
        anonymous_map_options.build()?;
    }

    Ok(())
}

fn init_aux_vec(
    elf: &ElfHeaders,
    elf_map_range: &RelocatedRange,
    ldso_base: Option<Vaddr>,
) -> Result<AuxVec> {
    let mut aux_vec = AuxVec::new();

    aux_vec.set(AuxKey::AT_PAGESZ, PAGE_SIZE as _)?;

    let Some(ph_vaddr) = elf_map_range.relocated_addr_of(elf.find_vaddr_of_phdrs()?) else {
        return_errno_with_message!(
            Errno::ENOEXEC,
            "the ELF program headers are not located in any segments"
        );
    };
    aux_vec.set(AuxKey::AT_PHDR, ph_vaddr as u64)?;
    aux_vec.set(AuxKey::AT_PHNUM, elf.ph_count() as u64)?;
    aux_vec.set(AuxKey::AT_PHENT, elf.ph_ent() as u64)?;

    let Some(entry_vaddr) = elf_map_range.relocated_addr_of(elf.entry_point()) else {
        return_errno_with_message!(
            Errno::ENOEXEC,
            "the entry point is not located in any segments"
        );
    };
    aux_vec.set(AuxKey::AT_ENTRY, entry_vaddr as u64)?;

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
