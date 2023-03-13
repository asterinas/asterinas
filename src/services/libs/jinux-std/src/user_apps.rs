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
        "PATH=/bin",
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
