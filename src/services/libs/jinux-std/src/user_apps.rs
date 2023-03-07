use crate::prelude::*;

pub struct UserApp {
    pub executable_path: String,
    pub argv: Vec<CString>,
    pub envp: Vec<CString>,
}

impl UserApp {
    pub fn new(executable_path: &str) -> Result<Self> {
        let app_name = String::from(executable_path);
        let arg0 = CString::new(executable_path)?;
        Ok(UserApp {
            executable_path: app_name,
            argv: vec![arg0],
            envp: Vec::new(),
        })
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
    let argv = ["sh", "-l"];
    let envp = [
        "SHELL=/bin/sh",
        "PWD=/",
        "LOGNAME=root",
        "HOME=/",
        "USER=root",
        "PATH=",
        "OLDPWD=/",
    ];

    let mut argv = to_vec_cstring(&argv)?;
    let mut envp = to_vec_cstring(&envp)?;
    busybox.argv.append(&mut argv);
    busybox.envp.append(&mut envp);
    Ok(busybox)
}

fn to_vec_cstring(raw_strs: &[&str]) -> Result<Vec<CString>> {
    let mut res = Vec::new();
    for raw_str in raw_strs {
        let cstring = CString::new(*raw_str)?;
        res.push(cstring);
    }
    Ok(res)
}
