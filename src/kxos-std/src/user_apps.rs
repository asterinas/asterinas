use crate::prelude::*;

pub struct UserApp {
    app_name: CString,
    app_content: &'static [u8],
}

impl UserApp {
    pub fn new(app_name: &str, app_content: &'static [u8]) -> Self {
        let app_name = CString::new(app_name).unwrap();
        UserApp {
            app_name,
            app_content,
        }
    }

    pub fn app_name(&self) -> CString {
        self.app_name.clone()
    }

    pub fn app_content(&self) -> &'static [u8] {
        self.app_content
    }
}

pub fn get_all_apps() -> Vec<UserApp> {
    let mut res = Vec::new();

    // Most simple hello world, written in assembly
    let app1 = UserApp::new("hello_world", read_hello_world_content());
    res.push(app1);

    // Hello world, written in C language.
    // Since glibc requires the app name starts with "/", and we don't have filesystem now.
    // So we manually add a leading "/" for app written in C language.
    let app2 = UserApp::new("/hello_c", read_hello_c_content());
    res.push(app2);

    // Fork process, written in assembly
    let app3 = UserApp::new("fork", read_fork_content());
    res.push(app3);

    // Execve, written in C language.
    let app4 = UserApp::new("/execve", read_execve_content());
    res.push(app4);

    res
}

fn read_hello_world_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/hello_world/hello_world")
}

fn read_hello_c_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/hello_c/hello")
}

fn read_fork_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/fork/fork")
}

fn read_execve_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/execve/execve")
}

pub fn read_execve_hello_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/execve/hello")
}
