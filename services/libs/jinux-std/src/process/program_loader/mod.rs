pub mod elf;
mod shebang;

use crate::fs::fs_resolver::{FsPath, FsResolver, AT_FDCWD};
use crate::fs::utils::Dentry;
use crate::prelude::*;
use crate::vm::vmar::Vmar;
use jinux_rights::Full;

use self::elf::{load_elf_to_root_vmar, ElfLoadInfo};
use self::shebang::parse_shebang_line;

/// Load an executable to root vmar, including loading programe image, preparing heap and stack,
/// initializing argv, envp and aux tables.
/// About recursion_limit: recursion limit is used to limit th recursion depth of shebang executables.
/// If the interpreter(the program behind #!) of shebang executable is also a shebang,
/// then it will trigger recursion. We will try to setup root vmar for the interpreter.
/// I guess for most cases, setting the recursion_limit as 1 should be enough.
/// because the interpreter is usually an elf binary(e.g., /bin/bash)
pub fn load_program_to_root_vmar(
    root_vmar: &Vmar<Full>,
    elf_file: Arc<Dentry>,
    argv: Vec<CString>,
    envp: Vec<CString>,
    fs_resolver: &FsResolver,
    recursion_limit: usize,
) -> Result<ElfLoadInfo> {
    let abs_path = elf_file.abs_path();
    let vnode = elf_file.vnode();
    let file_header = {
        // read the first page of file header
        let mut file_header_buffer = Box::new([0u8; PAGE_SIZE]);
        vnode.read_at(0, &mut *file_header_buffer)?;
        file_header_buffer
    };
    if let Some(mut new_argv) = parse_shebang_line(&*file_header)? {
        if recursion_limit == 0 {
            return_errno_with_message!(Errno::EINVAL, "the recursieve limit is reached");
        }
        new_argv.extend_from_slice(&argv);
        let interpreter = {
            let filename = new_argv[0].to_str()?.to_string();
            let fs_path = FsPath::new(AT_FDCWD, &filename)?;
            fs_resolver.lookup(&fs_path)?
        };
        return load_program_to_root_vmar(
            root_vmar,
            interpreter,
            new_argv,
            envp,
            fs_resolver,
            recursion_limit - 1,
        );
    }
    let elf_load_info =
        load_elf_to_root_vmar(root_vmar, &*file_header, elf_file, fs_resolver, argv, envp)?;
    Ok(elf_load_info)
}
