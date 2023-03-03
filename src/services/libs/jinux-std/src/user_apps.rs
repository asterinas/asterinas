use crate::fs::{
    fs_resolver::{FsPath, FsResolver},
    utils::AccessMode,
};
use crate::prelude::*;

pub struct UserApp {
    pub elf_path: CString,
    pub app_content: Vec<u8>,
    pub argv: Vec<CString>,
    pub envp: Vec<CString>,
}

impl UserApp {
    pub fn new(elf_path: &str) -> Result<Self> {
        let app_name = CString::new(elf_path).unwrap();
        let app_content = {
            let fs = FsResolver::new();
            let file = fs.open(&FsPath::try_from(elf_path)?, AccessMode::O_RDONLY as u32, 0)?;
            let mut content = Vec::new();
            let len = file.read_to_end(&mut content)?;
            if len != file.len() {
                return_errno_with_message!(Errno::EINVAL, "read len is not equal to file size");
            }
            content
        };
        Ok(UserApp {
            elf_path: app_name,
            app_content,
            argv: Vec::new(),
            envp: Vec::new(),
        })
    }

    pub fn set_argv(&mut self, argv: Vec<CString>) {
        self.argv = argv;
    }

    pub fn set_envp(&mut self, envp: Vec<CString>) {
        self.envp = envp;
    }
}

pub fn get_all_apps() -> Result<Vec<UserApp>> {
    let mut res = Vec::with_capacity(16);

    // Most simple hello world, written in assembly
    let asm_hello_world = UserApp::new("hello_world/hello_world")?;
    res.push(asm_hello_world);

    // Hello world, written in C language.
    // Since glibc requires the elf path starts with "/", and we don't have filesystem now.
    // So we manually add a leading "/" for app written in C language.
    let hello_c = UserApp::new("/hello_c/hello")?;
    res.push(hello_c);

    // Fork process, written in assembly
    let asm_fork = UserApp::new("fork/fork")?;
    res.push(asm_fork);

    // Execve, written in C language.
    let execve_c = UserApp::new("/execve/execve")?;
    res.push(execve_c);

    // Fork new process, written in C language. (Fork in glibc uses syscall clone actually)
    let fork_c = UserApp::new("/fork_c/fork")?;
    res.push(fork_c);

    // signal test
    let signal_test = UserApp::new("/signal_c/signal_test")?;
    res.push(signal_test);

    // pthread test
    let pthread_test = UserApp::new("/pthread/pthread_test")?;
    res.push(pthread_test);

    Ok(res)
}

pub fn get_busybox_app() -> Result<UserApp> {
    // busybox
    let mut busybox = UserApp::new("/busybox/busybox")?;
    // -l option means the busybox is running as logging shell
    let argv = ["/busybox", "sh", "-l"];
    let envp = [
        "SHELL=/bin/sh",
        "PWD=/",
        "LOGNAME=root",
        "HOME=/",
        "USER=root",
        "PATH=",
        "OLDPWD=/",
    ];

    let argv = to_vec_cstring(&argv)?;
    let envp = to_vec_cstring(&envp)?;
    busybox.set_argv(argv);
    busybox.set_envp(envp);
    Ok(busybox)
}

fn read_execve_content() -> &'static [u8] {
    include_bytes!("../../../../apps/execve/execve")
}

pub fn read_execve_hello_content() -> &'static [u8] {
    include_bytes!("../../../../apps/execve/hello")
}

fn to_vec_cstring(raw_strs: &[&str]) -> Result<Vec<CString>> {
    let mut res = Vec::new();
    for raw_str in raw_strs {
        let cstring = CString::new(*raw_str)?;
        res.push(cstring);
    }
    Ok(res)
}
