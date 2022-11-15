use crate::prelude::*;

pub struct UserApp {
    pub app_name: CString,
    pub app_content: &'static [u8],
    pub argv: Vec<CString>,
    pub envp: Vec<CString>,
}

impl UserApp {
    pub fn new(app_name: &str, app_content: &'static [u8]) -> Self {
        let app_name = CString::new(app_name).unwrap();
        UserApp {
            app_name,
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
    // Since glibc requires the app name starts with "/", and we don't have filesystem now.
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

    // busybox
    let mut busybox = UserApp::new("/busybox", read_busybox_content());
    let argv = ["./busybox", "sh"];
    let envp = ["SHELL=/bin/bash", "COLORTERM=truecolor", "TERM_PROGRAM_VERSION=1.73.0", "LC_ADDRESS=zh_CN.UTF-8", "LC_NAME=zh_CN.UTF-8", "LC_MONETARY=zh_CN.UTF-8", "PWD=/", "LOGNAME=jiangjf", "XDG_SESSION_TYPE=tty", "VSCODE_GIT_ASKPASS_NODE=/home/jiangjf/.vscode-server/bin/8fa188b2b301d36553cbc9ce1b0a146ccb93351f/node", "MOTD_SHOWN=pam", "HOME=/home/jiangjf", "LC_PAPER=zh_CN.UTF-8", "LANG=en_US.UTF-8", "LS_COLORS=rs=0:di=01;34:ln=01;36:mh=00:pi=40;33:so=01;35:do=01;35:bd=40;33;01:cd=40;33;01:or=40;31;01:mi=00:su=37;41:sg=30;43:ca=30;41:tw=30;42:ow=34;42:st=37;44:ex=01;32:*.tar=01;31:*.tgz=01;31:*.arc=01;31:*.arj=01;31:*.taz=01;31:*.lha=01;31:*.lz4=01;31:*.lzh=01;31:*.lzma=01;31:*.tlz=01;31:*.txz=01;31:*.tzo=01;31:*.t7z=01;31:*.zip=01;31:*.z=01;31:*.dz=01;31:*.gz=01;31:*.lrz=01;31:*.lz=01;31:*.lzo=01;31:*.xz=01;31:*.zst=01;31:*.tzst=01;31:*.bz2=01;31:*.bz=01;31:*.tbz=01;31:*.tbz2=01;31:*.tz=01;31:*.deb=01;31:*.rpm=01;31:*.jar=01;31:*.war=01;31:*.ear=01;31:*.sar=01;31:*.rar=01;31:*.alz=01;31:*.ace=01;31:*.zoo=01;31:*.cpio=01;31:*.7z=01;31:*.rz=01;31:*.cab=01;31:*.wim=01;31:*.swm=01;31:*.dwm=01;31:*.esd=01;31:*.jpg=01;35:*.jpeg=01;35:*.mjpg=01;35:*.mjpeg=01;35:*.gif=01;35:*.bmp=01;35:*.pbm=01;35:*.pgm=01;35:*.ppm=01;35:*.tga=01;35:*.xbm=01;35:*.xpm=01;35:*.tif=01;35:*.tiff=01;35:*.png=01;35:*.svg=01;35:*.svgz=01;35:*.mng=01;35:*.pcx=01;35:*.mov=01;35:*.mpg=01;35:*.mpeg=01;35:*.m2v=01;35:*.mkv=01;35:*.webm=01;35", "GIT_ASKPASS=/home/jiangjf/.vscode-server/bin/8fa188b2b301d36553cbc9ce1b0a146ccb93351f/extensions/git/dist/askpass.sh", "SSH_CONNECTION=30.177.3.156 54687 30.77.178.76 22", "VSCODE_GIT_ASKPASS_EXTRA_ARGS=", "LESSCLOSE=/usr/bin/lesspipe %s %s", "XDG_SESSION_CLASS=user", "TERM=xterm-256color", "LC_IDENTIFICATION=zh_CN.UTF-8", "LESSOPEN=| /usr/bin/lesspipe %s", "USER=jiangjf", "VSCODE_GIT_IPC_HANDLE=/run/user/1015/vscode-git-623b69fb06.sock", "SHLVL=2", "LC_TELEPHONE=zh_CN.UTF-8", "LC_MEASUREMENT=zh_CN.UTF-8", "XDG_SESSION_ID=8884", "XDG_RUNTIME_DIR=/run/user/1015", "SSH_CLIENT=30.177.3.156 54687 22", "LC_TIME=zh_CN.UTF-8", "VSCODE_GIT_ASKPASS_MAIN=/home/jiangjf/.vscode-server/bin/8fa188b2b301d36553cbc9ce1b0a146ccb93351f/extensions/git/dist/askpass-main.js", "XDG_DATA_DIRS=/usr/local/share:/usr/share:/var/lib/snapd/desktop", "BROWSER=/home/jiangjf/.vscode-server/bin/8fa188b2b301d36553cbc9ce1b0a146ccb93351f/bin/helpers/browser.sh", "PATH=/home/jiangjf/.vscode-server/bin/8fa188b2b301d36553cbc9ce1b0a146ccb93351f/bin/remote-cli:/home/jiangjf/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games:/usr/local/games:/snap/bin", "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1015/bus", "LC_NUMERIC=zh_CN.UTF-8", "TERM_PROGRAM=vscode", "VSCODE_IPC_HOOK_CLI=/run/user/1015/vscode-ipc-ed06ed64-441d-4b59-a8fe-90ce2cf29a8a.sock", "OLDPWD=/"];
    let argv = to_vec_cstring(&argv).unwrap();
    let envp = to_vec_cstring(&envp).unwrap();
    busybox.set_argv(argv);
    busybox.set_envp(envp);
    res.push(busybox);

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

fn read_fork_c_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/fork_c/fork")
}

fn read_signal_test_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/signal_c/signal_test")
}

fn read_busybox_content() -> &'static [u8] {
    include_bytes!("../../kxos-user/busybox/busybox")
}

fn to_vec_cstring(raw_strs: &[&str]) -> Result<Vec<CString>> {
    let mut res = Vec::new();
    for raw_str in raw_strs {
        let cstring = CString::new(*raw_str)?;
        res.push(cstring);
    }
    Ok(res)
}
