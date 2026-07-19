// SPDX-License-Identifier: MPL-2.0

pub(super) mod elf;
mod shebang;

use self::{
    elf::{ElfHeaders, ElfLoadInfo, load_elf_to_vmar},
    shebang::parse_shebang_line,
};
use super::execve::ShebangScriptPath;
use crate::{
    fs::{
        file::{InodeType, Permission},
        vfs::{
            inode::Inode,
            path::{FsPath, Path, PathResolver},
        },
    },
    prelude::*,
    vm::vmar::Vmar,
};

/// Represents an executable file that is ready to be loaded into memory and executed.
///
/// This struct encapsulates the ELF file to be executed along with its header data,
/// the `argv` and the `envp` which is required for the program execution.
pub(super) struct ProgramToLoad {
    elf_file: Path,
    elf_headers: ElfHeaders,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl ProgramToLoad {
    /// Constructs a new `ProgramToLoad` from a file and handles shebang interpretation if
    /// necessary.
    pub(super) fn build_from_file(
        mut elf_file: Path,
        path_resolver: &PathResolver,
        mut script_path: ShebangScriptPath,
        mut argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Self> {
        check_executable_inode(elf_file.inode().as_ref())?;

        // A limit to the recursion depth of shebang executables.
        //
        // If the interpreter is a shebang, then recursion will be triggered. If it loops, we
        // should fail. We follow the same limit as Linux.
        let mut recursive_limit = 5;

        let (file_first_page, len) = loop {
            // Read the first page of the file, which should contain a shebang or an ELF header.
            let (file_first_page, len) = {
                let mut buffer = Box::new([0u8; PAGE_SIZE]);
                let len = elf_file.inode().read_bytes_at(0, &mut *buffer)?;
                (buffer, len)
            };

            let Some(mut new_argv) = parse_shebang_line(&file_first_page[..len])? else {
                break (file_first_page, len);
            };

            // A shebang interpreter must reopen the original script path after exec. A path
            // synthesized from a close-on-exec file descriptor will no longer be accessible then,
            // so fail before changing the process state. Linux does the same through
            // `BINPRM_FLAGS_PATH_INACCESSIBLE` in `fs/binfmt_script.c`.
            let current_script_path = match script_path {
                ShebangScriptPath::Accessible(path) => path,
                ShebangScriptPath::Inaccessible => {
                    return_errno_with_message!(
                        Errno::ENOENT,
                        "the script path is inaccessible after closing close-on-exec file descriptors"
                    );
                }
            };

            if recursive_limit == 0 {
                return_errno_with_message!(Errno::ELOOP, "the recursieve limit is reached");
            }
            recursive_limit -= 1;

            let interpreter_filename = new_argv[0].clone();
            let interpreter = {
                let filename = interpreter_filename.to_str()?;
                let fs_path = FsPath::try_from(filename)?;
                path_resolver.lookup(&fs_path)?
            };
            check_executable_inode(interpreter.inode().as_ref())?;

            // Update the argument list and the executable inode. Then, try again.
            //
            // The interpreter receives the script path as its first argument,
            // regardless of the caller-supplied `argv[0]`. For example,
            // `exec -a custom ./script arg` with `#!/bin/sh` must execute
            // `/bin/sh ./script arg`, not `/bin/sh custom arg`.
            new_argv.push(current_script_path);
            new_argv.extend(argv.into_iter().skip(1));
            argv = new_argv;
            script_path = ShebangScriptPath::Accessible(interpreter_filename);
            elf_file = interpreter;
        };

        let elf_headers = ElfHeaders::parse(&file_first_page[..len])?;

        Ok(Self {
            elf_file,
            elf_headers,
            argv,
            envp,
        })
    }

    /// Returns the ELF file that will be loaded.
    pub(super) fn elf_file(&self) -> &Path {
        &self.elf_file
    }

    /// Loads the executable into the specified virtual memory space.
    ///
    /// Returns the information about the ELF loading process.
    pub(super) fn load_to_vmar(
        self,
        vmar: &Vmar,
        path_resolver: &PathResolver,
    ) -> Result<ElfLoadInfo> {
        let elf_load_info = load_elf_to_vmar(
            vmar,
            self.elf_file,
            path_resolver,
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
