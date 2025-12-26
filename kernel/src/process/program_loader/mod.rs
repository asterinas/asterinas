// SPDX-License-Identifier: MPL-2.0

pub(super) mod elf;
mod shebang;

use self::{
    elf::{ElfHeaders, ElfLoadInfo, load_elf_to_vmar},
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
pub(super) struct ProgramToLoad {
    elf_inode: Arc<dyn Inode>,
    elf_headers: ElfHeaders,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl ProgramToLoad {
    /// Constructs a new `ProgramToLoad` from an inode and handles shebang interpretation if
    /// necessary.
    pub(super) fn build_from_inode(
        mut elf_inode: Arc<dyn Inode>,
        fs_resolver: &FsResolver,
        mut argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Self> {
        check_executable_inode(elf_inode.as_ref())?;

        // A limit to the recursion depth of shebang executables.
        //
        // If the interpreter is a shebang, then recursion will be triggered. If it loops, we
        // should fail. We follow the same limit as Linux.
        let mut recursive_limit = 5;

        let (file_first_page, len) = loop {
            // Read the first page of the file, which should contain a shebang or an ELF header.
            let (file_first_page, len) = {
                let mut buffer = Box::new([0u8; PAGE_SIZE]);
                let len = elf_inode.read_bytes_at(0, &mut *buffer)?;
                (buffer, len)
            };

            let Some(mut new_argv) = parse_shebang_line(&file_first_page[..len])? else {
                break (file_first_page, len);
            };

            if recursive_limit == 0 {
                return_errno_with_message!(Errno::ELOOP, "the recursieve limit is reached");
            }
            recursive_limit -= 1;

            let interpreter = {
                let filename = new_argv[0].to_str()?.to_string();
                let fs_path = FsPath::try_from(filename.as_str())?;
                fs_resolver.lookup(&fs_path)?
            };
            check_executable_inode(interpreter.inode().as_ref())?;

            // Update the argument list and the executable inode. Then, try again.
            new_argv.extend(argv);
            argv = new_argv;
            elf_inode = interpreter.inode().clone();
        };

        let elf_headers = ElfHeaders::parse(&file_first_page[..len])?;

        Ok(Self {
            elf_inode,
            elf_headers,
            argv,
            envp,
        })
    }

    /// Loads the executable into the specified virtual memory space.
    ///
    /// Returns the information about the ELF loading process.
    pub(super) fn load_to_vmar(self, vmar: &Vmar, fs_resolver: &FsResolver) -> Result<ElfLoadInfo> {
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

fn check_executable_inode(inode: &dyn Inode) -> Result<()> {
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
