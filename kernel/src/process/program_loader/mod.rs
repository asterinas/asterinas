// SPDX-License-Identifier: MPL-2.0

pub mod elf;
mod shebang;

use self::{
    elf::{load_elf_to_vm, ElfHeaders, ElfLoadInfo},
    shebang::parse_shebang_line,
};
use super::process_vm::ProcessVm;
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver, AT_FDCWD},
        path::Dentry,
        utils::{InodeType, Permission},
    },
    prelude::*,
};

/// Represents an executable file that is ready to be loaded into memory and executed.
///
/// This struct encapsulates the ELF file to be executed along with its header data,
/// the `argv` and the `envp` which is required for the program execution.
pub struct ProgramToLoad {
    elf_file: Dentry,
    file_first_page: Box<[u8; PAGE_SIZE]>,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl ProgramToLoad {
    /// Constructs a new `ProgramToLoad` from a file, handling shebang interpretation if needed.
    ///
    /// About `recursion_limit`: recursion limit is used to limit th recursion depth of shebang executables.
    /// If the interpreter(the program behind #!) of shebang executable is also a shebang,
    /// then it will trigger recursion. We will try to setup root vmar for the interpreter.
    /// I guess for most cases, setting the `recursion_limit` as 1 should be enough.
    /// because the interpreter is usually an elf binary(e.g., /bin/bash)
    pub fn build_from_file(
        elf_file: Dentry,
        fs_resolver: &FsResolver,
        argv: Vec<CString>,
        envp: Vec<CString>,
        recursion_limit: usize,
    ) -> Result<Self> {
        let inode = elf_file.inode();
        let file_first_page = {
            // Read the first page of file header, which must contain the ELF header.
            let mut buffer = Box::new([0u8; PAGE_SIZE]);
            inode.read_bytes_at(0, &mut *buffer)?;
            buffer
        };
        if let Some(mut new_argv) = parse_shebang_line(&*file_first_page)? {
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
            return Self::build_from_file(
                interpreter,
                fs_resolver,
                new_argv,
                envp,
                recursion_limit - 1,
            );
        }

        Ok(Self {
            elf_file,
            file_first_page,
            argv,
            envp,
        })
    }

    /// Loads the executable into the specified virtual memory space.
    ///
    /// Returns a tuple containing:
    /// 1. The absolute path of the loaded executable.
    /// 2. Information about the ELF loading process.
    pub fn load_to_vm(
        self,
        process_vm: &ProcessVm,
        fs_resolver: &FsResolver,
    ) -> Result<(String, ElfLoadInfo)> {
        let abs_path = self.elf_file.abs_path();
        let elf_headers = ElfHeaders::parse_elf(&*self.file_first_page)?;
        let elf_load_info = load_elf_to_vm(
            process_vm,
            self.elf_file,
            fs_resolver,
            elf_headers,
            self.argv,
            self.envp,
        )?;

        Ok((abs_path, elf_load_info))
    }
}

pub fn check_executable_file(dentry: &Dentry) -> Result<()> {
    if dentry.type_().is_directory() {
        return_errno_with_message!(Errno::EISDIR, "the file is a directory");
    }

    if dentry.type_() == InodeType::SymLink {
        return_errno_with_message!(Errno::ELOOP, "the file is a symbolic link");
    }

    if !dentry.type_().is_regular_file() {
        return_errno_with_message!(Errno::EACCES, "the dentry is not a regular file");
    }

    if dentry
        .inode()
        .check_permission(Permission::MAY_EXEC)
        .is_err()
    {
        return_errno_with_message!(Errno::EACCES, "the dentry is not executable");
    }

    Ok(())
}
