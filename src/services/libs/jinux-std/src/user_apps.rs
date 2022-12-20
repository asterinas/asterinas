use crate::prelude::*;

pub struct UserApp {
    pub elf_path: CString,
    pub app_content: &'static [u8],
    pub argv: Vec<CString>,
    pub envp: Vec<CString>,
}

impl UserApp {
    pub fn new(elf_path: &str, app_content: &'static [u8]) -> Self {
        let app_name = CString::new(elf_path).unwrap();
        UserApp {
            elf_path: app_name,
            app_content,
            argv: Vec::new(),
            envp: Vec::new(),
        }
    }

    pub fn set_argv(&mut self, argv: Vec<CString>) {
        self.argv = argv;
    }

    pub fn set_envp(&mut self, envp: Vec<CString>) {
        self.envp = envp;
    }
}

pub fn get_all_apps() -> Vec<UserApp> {
    let mut res = Vec::with_capacity(16);

    // Most simple hello world, written in assembly
    let asm_hello_world = UserApp::new("hello_world", read_hello_world_content());
    res.push(asm_hello_world);

    // Hello world, written in C language.
    // Since glibc requires the elf path starts with "/", and we don't have filesystem now.
    // So we manually add a leading "/" for app written in C language.
    let hello_c = UserApp::new("/hello_c", read_hello_c_content());
    res.push(hello_c);

    // Fork process, written in assembly
    let asm_fork = UserApp::new("fork", read_fork_content());
    res.push(asm_fork);

    // Execve, written in C language.
    let execve_c = UserApp::new("/execve", read_execve_content());
    res.push(execve_c);

    // Fork new process, written in C language. (Fork in glibc uses syscall clone actually)
    let fork_c = UserApp::new("/fork", read_fork_c_content());
    res.push(fork_c);

    // signal test
    let signal_test = UserApp::new("/signal_test", read_signal_test_content());
    res.push(signal_test);

    // pthread test
    let pthread_test = UserApp::new("/pthread_test", read_pthread_test_content());
    res.push(pthread_test);

    res
}

pub fn get_busybox_app() -> UserApp {
    // busybox
    let mut busybox = UserApp::new("/busybox", read_busybox_content());
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

    let argv = to_vec_cstring(&argv).unwrap();
    let envp = to_vec_cstring(&envp).unwrap();
    busybox.set_argv(argv);
    busybox.set_envp(envp);
    busybox
}

fn read_hello_world_content() -> &'static [u8] {
    include_bytes!("../../../../apps/hello_world/hello_world")
}

fn read_hello_c_content() -> &'static [u8] {
    include_bytes!("../../../../apps/hello_c/hello")
}

fn read_fork_content() -> &'static [u8] {
    include_bytes!("../../../../apps/fork/fork")
}

fn read_execve_content() -> &'static [u8] {
    include_bytes!("../../../../apps/execve/execve")
}

pub fn read_execve_hello_content() -> &'static [u8] {
    include_bytes!("../../../../apps/execve/hello")
}

fn read_fork_c_content() -> &'static [u8] {
    include_bytes!("../../../../apps/fork_c/fork")
}

fn read_signal_test_content() -> &'static [u8] {
    include_bytes!("../../../../apps/signal_c/signal_test")
}

fn read_pthread_test_content() -> &'static [u8] {
    include_bytes!("../../../../apps/pthread/pthread_test")
}

fn read_busybox_content() -> &'static [u8] {
    include_bytes!("../../../../apps/busybox/busybox")
}

fn to_vec_cstring(raw_strs: &[&str]) -> Result<Vec<CString>> {
    let mut res = Vec::new();
    for raw_str in raw_strs {
        let cstring = CString::new(*raw_str)?;
        res.push(cstring);
    }
    Ok(res)
}
