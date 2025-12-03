// SPDX-License-Identifier: MPL-2.0

pub mod elf;
mod shebang;

use self::{
    elf::{load_elf_to_vmar, ElfHeaders, ElfLoadInfo},
    shebang::parse_shebang_line,
};
use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        utils::{Inode, InodeType, Permission},
    },
    prelude::*,
    vm::vmar::Vmar,
};

/// Represents an executable file that is ready to be loaded into memory and executed.
///
/// This struct encapsulates the ELF file to be executed along with its header data,
/// the `argv` and the `envp` which is required for the program execution.
pub struct ProgramToLoad {
    elf_inode: Arc<dyn Inode>,
    elf_headers: ElfHeaders,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl ProgramToLoad {
    /// Constructs a new `ProgramToLoad` from a file, handling shebang interpretation if needed.
    ///
    /// About `recursion_limit`: recursion limit is used to limit th recursion depth of shebang executables.
    /// If the interpreter(the program behind #!) of shebang executable is also a shebang,
    /// then it will trigger recursion. We will try to setup VMAR for the interpreter.
    /// I guess for most cases, setting the `recursion_limit` as 1 should be enough.
    /// because the interpreter is usually an elf binary(e.g., /bin/bash)
    pub fn build_from_inode(
        elf_inode: &Arc<dyn Inode>,
        fs_resolver: &FsResolver,
        argv: Vec<CString>,
        envp: Vec<CString>,
        recursion_limit: usize,
    ) -> Result<Self> {
        let file_first_page = {
            // Read the first page of file header, which must contain the ELF header.
            let mut buffer = Box::new([0u8; PAGE_SIZE]);
            elf_inode.read_bytes_at(0, &mut *buffer)?;
            buffer
        };
        if let Some(mut new_argv) = parse_shebang_line(&*file_first_page)? {
            if recursion_limit == 0 {
                return_errno_with_message!(Errno::ELOOP, "the recursieve limit is reached");
            }
            new_argv.extend_from_slice(&argv);
            let interpreter = {
                let filename = new_argv[0].to_str()?.to_string();
                let fs_path = FsPath::try_from(filename.as_str())?;
                fs_resolver.lookup_inode(&fs_path)?
            };
            check_executable_inode(interpreter.inode())?;
            return Self::build_from_inode(
                interpreter.inode(),
                fs_resolver,
                new_argv,
                envp,
                recursion_limit - 1,
            );
        }

        let elf_headers = ElfHeaders::parse_elf(&*file_first_page)?;

        Ok(Self {
            elf_inode: elf_inode.clone(),
            elf_headers,
            argv,
            envp,
        })
    }

    /// Loads the executable into the specified virtual memory space.
    ///
    /// Returns the information about the ELF loading process.
    pub fn load_to_vmar(self, vmar: &Vmar, fs_resolver: &FsResolver) -> Result<ElfLoadInfo> {
        let elf_load_info = load_elf_to_vmar(
            vmar,
            &self.elf_inode,
            fs_resolver,
            self.elf_headers,
            self.argv,
            self.envp,
        )?;

        Ok(elf_load_info)
    }
}

pub fn check_executable_inode(inode: &Arc<dyn Inode>) -> Result<()> {
    if inode.type_().is_directory() {
        return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
    }

    if inode.type_() == InodeType::SymLink {
        return_errno_with_message!(Errno::ELOOP, "the inode is a symbolic link");
    }

    if !inode.type_().is_regular_file() {
        return_errno_with_message!(Errno::EACCES, "the inode is not a regular file");
    }

    if inode.check_permission(Permission::MAY_EXEC).is_err() {
        return_errno_with_message!(Errno::EACCES, "the inode is not executable");
    }

    Ok(())
}
