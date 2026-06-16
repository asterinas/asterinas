// SPDX-License-Identifier: MPL-2.0

pub(super) mod elf;
mod shebang;

use self::{
    elf::{ElfHeaders, ElfLoadInfo, load_elf_to_vmar},
    shebang::parse_shebang_line,
};
use crate::{
    fs::{
        file::{AccessMode, FileLike, InodeHandle, InodeType, Permission, StatusFlags},
        vfs::{
            inode::Inode,
            notify,
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
    elf_file: ExecutableFile,
    elf_headers: ElfHeaders,
    argv: Vec<CString>,
    envp: Vec<CString>,
}

/// Represents an opened executable file that tracks open/access/close notifications.
#[derive(Clone)]
pub struct ExecutableFile {
    file: Arc<dyn FileLike>,
}

impl ExecutableFile {
    /// Creates an executable file wrapper and emits an open notification.
    fn new(file: Arc<dyn FileLike>) -> Self {
        notify::on_open(&file);

        Self { file }
    }

    /// Returns the path of the executable file.
    pub(in crate::process) fn path(&self) -> &Path {
        self.file.path()
    }

    /// Reads bytes from the executable file at the specified offset.
    ///
    /// Emits an access notification when at least one byte is read.
    pub(in crate::process) fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let read_len = self.file.read_bytes_at(offset, buf)?;
        if read_len > 0 {
            notify::on_access(&self.file);
        }

        Ok(read_len)
    }

    /// Returns the inner file and consume this handle.
    pub(in crate::process) fn into_file(self) -> Arc<dyn FileLike> {
        self.file
    }
}

impl ProgramToLoad {
    /// Constructs a new `ProgramToLoad` from an opened executable and handles shebang
    /// interpretation if necessary.
    pub(super) fn build_from_file(
        mut elf_file: ExecutableFile,
        path_resolver: &PathResolver,
        mut argv: Vec<CString>,
        envp: Vec<CString>,
    ) -> Result<Self> {
        // A limit to the recursion depth of shebang executables.
        //
        // If the interpreter is a shebang, then recursion will be triggered. If it loops, we
        // should fail. We follow the same limit as Linux.
        let mut recursive_limit = 5;

        let (file_first_page, len) = loop {
            // Read the first page of the file, which should contain a shebang or an ELF header.
            let (file_first_page, len) = {
                let mut buffer = Box::new([0u8; PAGE_SIZE]);
                let len = elf_file.read_bytes_at(0, &mut *buffer)?;
                (buffer, len)
            };

            let Some(mut new_argv) = parse_shebang_line(&file_first_page[..len])? else {
                break (file_first_page, len);
            };

            if recursive_limit == 0 {
                return_errno_with_message!(Errno::ELOOP, "the recursive limit is reached");
            }
            recursive_limit -= 1;

            let interpreter = {
                let filename = new_argv[0].to_str()?.to_string();
                let fs_path = FsPath::try_from(filename.as_str())?;
                path_resolver.lookup(&fs_path)?
            };

            // Update the argument list and the executable file. Then, try again.
            new_argv.extend(argv);
            argv = new_argv;
            elf_file = open_executable_file(interpreter)?;
        };

        let elf_headers = ElfHeaders::parse(&file_first_page[..len])?;

        Ok(Self {
            elf_file,
            elf_headers,
            argv,
            envp,
        })
    }

    /// Loads the executable into the specified virtual memory space.
    ///
    /// Returns the information about the ELF loading process.
    pub(super) fn load_to_vmar(
        self,
        vmar: &Vmar,
        path_resolver: &PathResolver,
    ) -> Result<ElfLoadInfo> {
        load_elf_to_vmar(
            vmar,
            self.elf_file,
            path_resolver,
            self.elf_headers,
            self.argv,
            self.envp,
        )
    }
}

/// Opens a path as an executable file.
pub fn open_executable_file(path: Path) -> Result<ExecutableFile> {
    check_executable_inode(path.inode().as_ref())?;

    let file: Arc<dyn FileLike> = Arc::new(InodeHandle::new_unchecked_access(
        path,
        AccessMode::O_RDONLY,
        StatusFlags::empty(),
    )?);

    Ok(ExecutableFile::new(file))
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
