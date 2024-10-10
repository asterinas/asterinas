// SPDX-License-Identifier: MPL-2.0

pub mod elf;
mod shebang;

use self::{
    elf::{load_elf_to_vm, ElfLoadInfo},
    shebang::parse_shebang_line,
};
use super::process_vm::ProcessVm;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver, AT_FDCWD},
        path::Dentry,
    },
    prelude::*,
};

/// Load an executable to root vmar, including loading programme image, preparing heap and stack,
/// initializing argv, envp and aux tables.
/// About recursion_limit: recursion limit is used to limit th recursion depth of shebang executables.
/// If the interpreter(the program behind #!) of shebang executable is also a shebang,
/// then it will trigger recursion. We will try to setup root vmar for the interpreter.
/// I guess for most cases, setting the recursion_limit as 1 should be enough.
/// because the interpreter is usually an elf binary(e.g., /bin/bash)
pub fn load_program_to_vm(
    process_vm: &ProcessVm,
    elf_file: Dentry,
    argv: Vec<CString>,
    envp: Vec<CString>,
    fs_resolver: &FsResolver,
    recursion_limit: usize,
) -> Result<(String, ElfLoadInfo)> {
    let abs_path = elf_file.abs_path();
    let inode = elf_file.inode();
    let file_header = {
        // read the first page of file header
        let mut file_header_buffer = Box::new([0u8; PAGE_SIZE]);
        inode.read_bytes_at(0, &mut *file_header_buffer)?;
        file_header_buffer
    };
    if let Some(mut new_argv) = parse_shebang_line(&*file_header)? {
        if recursion_limit == 0 {
            return_errno_with_message!(Errno::ELOOP, "the recursieve limit is reached");
        }
        new_argv.extend_from_slice(&argv);
        let interpreter = {
            let filename = new_argv[0].to_str()?.to_string();
            let fs_path = FsPath::new(AT_FDCWD, &filename)?;
            fs_resolver.lookup(&fs_path)?
        };
        check_executable_file(&interpreter)?;
        return load_program_to_vm(
            process_vm,
            interpreter,
            new_argv,
            envp,
            fs_resolver,
            recursion_limit - 1,
        );
    }

    process_vm.clear_and_map();

    let elf_load_info =
        load_elf_to_vm(process_vm, &*file_header, elf_file, fs_resolver, argv, envp)?;

    Ok((abs_path, elf_load_info))
}

pub fn check_executable_file(dentry: &Dentry) -> Result<()> {
    if dentry.type_().is_directory() {
        return_errno_with_message!(Errno::EISDIR, "the file is a directory");
    }

    if !dentry.type_().is_regular_file() {
        return_errno_with_message!(Errno::EACCES, "the dentry is not a regular file");
    }

    if !dentry.mode()?.is_executable() {
        return_errno_with_message!(Errno::EACCES, "the dentry is not executable");
    }

    Ok(())
}
