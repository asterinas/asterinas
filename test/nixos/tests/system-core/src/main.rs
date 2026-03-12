// SPDX-License-Identifier: MPL-2.0

//! The test suite for system core applications on Asterinas NixOS.
//!
//! # Document maintenance
//!
//! An application's test suite and its "Verified Usage" section in Asterinas Book
//! should always be kept in sync.
//! So whenever you modify the test suite,
//! review the documentation and see if should be updated accordingly.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Shells
// ============================================================================

#[nixos_test]
fn zsh_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'echo \"Hello from Zsh\"' > /tmp/test_zsh.sh")?;
    nixos_shell.run_cmd_and_expect("zsh /tmp/test_zsh.sh", "Hello from Zsh")?;
    Ok(())
}

#[nixos_test]
fn fish_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'echo \"Hello from Fish\"' > /tmp/test_fish.fish")?;
    nixos_shell.run_cmd_and_expect("fish /tmp/test_fish.fish", "Hello from Fish")?;
    Ok(())
}

// ============================================================================
// System Monitoring
// ============================================================================

#[nixos_test]
fn fastfetch(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("fastfetch > /tmp/fastfetch-output.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/fastfetch-output.txt", "OS: Asterinas NixOS")?;
    Ok(())
}

// ============================================================================
// Coreutils
// ============================================================================

#[nixos_test]
fn coreutils_b2sum(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'hello' > /tmp/b2.txt")?;
    nixos_shell.run_cmd("b2sum /tmp/b2.txt > /tmp/checksums.b2")?;
    nixos_shell.run_cmd_and_expect("b2sum --check /tmp/checksums.b2", "OK")?;
    Ok(())
}

#[nixos_test]
fn coreutils_base64(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'hello' > /tmp/b64.txt")?;
    nixos_shell.run_cmd("base64 /tmp/b64.txt > /tmp/encoded.b64")?;
    nixos_shell.run_cmd_and_expect("base64 -d /tmp/encoded.b64", "hello")?;
    Ok(())
}

#[nixos_test]
fn coreutils_basename_dirname(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("basename /path/to/file.txt", "file.txt")?;
    nixos_shell.run_cmd_and_expect("dirname /path/to/file.txt", "/path/to")?;
    Ok(())
}

#[nixos_test]
fn coreutils_cat(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("printf 'Hello' > /tmp/hello.txt")?;
    nixos_shell.run_cmd("printf 'World' > /tmp/world.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/hello.txt /tmp/world.txt", "HelloWorld")?;
    Ok(())
}

#[nixos_test]
fn coreutils_chmod(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo '#!/bin/bash' > /tmp/script.sh")?;
    nixos_shell.run_cmd("chmod +x /tmp/script.sh")?;
    nixos_shell.run_cmd_and_expect("ls -l /tmp/script.sh", "x")?;
    Ok(())
}

#[nixos_test]
fn coreutils_cp_mv(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'test' > /tmp/source.txt")?;
    nixos_shell.run_cmd("cp /tmp/source.txt /tmp/copied.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/copied.txt", "test")?;
    nixos_shell.run_cmd("mv /tmp/copied.txt /tmp/moved.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/moved.txt", "test")?;
    Ok(())
}

#[nixos_test]
fn coreutils_echo_expr(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("echo \"Hello World\"", "Hello World")?;
    nixos_shell.run_cmd_and_expect("expr 2 + 3", "5")?;
    nixos_shell.run_cmd_and_expect(r"expr 10 \* 3", "30")?;
    Ok(())
}

#[nixos_test]
fn coreutils_head_tail(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("seq 0 2 20 > /tmp/numbers.txt")?;
    nixos_shell.run_cmd_and_expect("head -n 1 /tmp/numbers.txt", "0")?;
    nixos_shell.run_cmd_and_expect("tail -n 1 /tmp/numbers.txt", "20")?;
    Ok(())
}

#[nixos_test]
fn coreutils_ln(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'target content' > /tmp/target.txt")?;
    nixos_shell.run_cmd("ln -s /tmp/target.txt /tmp/symlink")?;
    nixos_shell.run_cmd_and_expect("readlink /tmp/symlink", "/tmp/target.txt")?;
    nixos_shell.run_cmd_and_expect("realpath /tmp/symlink", "/tmp/target.txt")?;
    Ok(())
}

#[nixos_test]
fn coreutils_mkdir_rm(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/nested/dir")?;
    nixos_shell.run_cmd_and_expect("ls -d /tmp/nested/dir", "/tmp/nested/dir")?;
    nixos_shell.run_cmd("rm -rf /tmp/nested")?;
    nixos_shell.run_cmd_and_expect("test -d /tmp/nested || echo 'deleted'", "deleted")?;
    Ok(())
}

#[nixos_test]
fn coreutils_od(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'A' > /tmp/binary.txt")?;
    nixos_shell.run_cmd_and_expect("od -c /tmp/binary.txt", "A")?;
    nixos_shell.run_cmd_and_expect("od -x /tmp/binary.txt", "0a")?;
    Ok(())
}

#[nixos_test]
fn coreutils_sha256sum(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'checksum test' > /tmp/checksum_test.txt")?;
    nixos_shell.run_cmd("sha256sum /tmp/checksum_test.txt > /tmp/checksums.sha256")?;
    nixos_shell.run_cmd_and_expect("sha256sum --check /tmp/checksums.sha256", "OK")?;
    Ok(())
}

#[nixos_test]
fn coreutils_stat(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("touch /tmp/stat_test.txt")?;
    nixos_shell.run_cmd_and_expect("stat -c \"%F\" /tmp/stat_test.txt", "regular empty file")?;
    Ok(())
}

#[nixos_test]
fn coreutils_touch_wc(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("touch -t 202601011200 /tmp/touched.txt")?;
    nixos_shell.run_cmd("echo 'one two three' > /tmp/wc_test.txt")?;
    nixos_shell.run_cmd_and_expect("wc -l /tmp/wc_test.txt", "1")?;
    nixos_shell.run_cmd_and_expect("wc -w /tmp/wc_test.txt", "3")?;
    nixos_shell.run_cmd_and_expect("wc -c /tmp/wc_test.txt", "14")?;
    Ok(())
}

#[nixos_test]
fn coreutils_csplit(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo -e 'section1\\n---\\nsection2' > /tmp/csplit_test.txt")?;
    nixos_shell.run_cmd("cd /tmp && csplit csplit_test.txt '/---/' '{*}'")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/xx00", "xx00")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/xx00", "section1")?;
    Ok(())
}

#[nixos_test]
fn coreutils_cut(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'root:x:0:0' > /tmp/cut_test.txt")?;
    nixos_shell.run_cmd_and_expect("cut -d':' -f1 /tmp/cut_test.txt", "root")?;
    nixos_shell.run_cmd_and_expect("cut -d':' -f1,3 /tmp/cut_test.txt", "root:0")?;
    nixos_shell.run_cmd_and_expect("cut -d':' -f1-3 /tmp/cut_test.txt", "root:x:0")?;
    Ok(())
}

#[nixos_test]
fn coreutils_dd(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'Hello DD Test' > /tmp/dd_input.txt")?;
    nixos_shell.run_cmd("dd if=/tmp/dd_input.txt of=/tmp/dd_output.txt 2>/dev/null")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/dd_output.txt", "Hello DD Test")?;
    nixos_shell.run_cmd("dd if=/dev/zero of=/tmp/disk.img bs=1M count=1 2>/dev/null")?;
    nixos_shell.run_cmd_and_expect("ls -l /tmp/disk.img", "disk.img")?;
    Ok(())
}

#[nixos_test]
fn coreutils_paste(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("printf 'a\\nb\\nc' > /tmp/paste1.txt")?;
    nixos_shell.run_cmd("printf '1\\n2\\n3' > /tmp/paste2.txt")?;
    nixos_shell.run_cmd_and_expect("paste /tmp/paste1.txt /tmp/paste2.txt", "a1")?;
    nixos_shell.run_cmd_and_expect("paste -d ',' /tmp/paste1.txt /tmp/paste2.txt", "a,1")?;
    Ok(())
}

#[nixos_test]
fn coreutils_sync(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("sync && echo 'completed'", "completed")?;
    Ok(())
}

// ============================================================================
// Util-linux
// ============================================================================

#[nixos_test]
fn util_linux_uname(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("uname -a", "Linux asterinas")?;
    Ok(())
}

#[nixos_test]
fn util_linux_df(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("df -h", "Filesystem")?;
    Ok(())
}

#[nixos_test]
fn util_linux_date_cal(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("date +\"%Y-%m-%d\"")?;
    nixos_shell.run_cmd_and_expect("cal 03 2026", "March 2026")?;
    Ok(())
}

#[nixos_test]
fn util_linux_id(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("id", "uid=0(root) gid=0(root) groups=0(root)")?;
    Ok(())
}

#[nixos_test]
fn util_linux_last(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("last", "still logged in")?;
    Ok(())
}

#[nixos_test]
fn util_linux_hexdump(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo -n 'A' > /tmp/hex_test.bin")?;
    nixos_shell.run_cmd_and_expect("hexdump -C /tmp/hex_test.bin", "41")?;
    Ok(())
}

#[nixos_test]
fn util_linux_whereis(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("whereis ls", "/bin/ls")?;
    Ok(())
}

// ============================================================================
// Procps
// ============================================================================

#[nixos_test]
fn procps_free(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("free -h", "Mem")?;
    Ok(())
}

#[nixos_test]
fn procps_ps(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("ps -eo ppid,cmd | head -1", "PPID CMD")?;
    nixos_shell.run_cmd_and_expect("pgrep -f systemd", "1")?;
    nixos_shell.run_cmd_and_expect("pmap 1 | head -1", "lib/systemd/systemd")?;
    Ok(())
}

#[nixos_test]
fn procps_uptime(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("uptime", "load average")?;
    Ok(())
}

// ============================================================================
// Findutils
// ============================================================================

#[nixos_test]
fn findutils_find(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/find_test")?;
    nixos_shell.run_cmd("touch /tmp/find_test/a.txt")?;
    nixos_shell.run_cmd("touch /tmp/find_test/B.TXT")?;
    nixos_shell.run_cmd("touch /tmp/find_test/c.log")?;
    nixos_shell.run_cmd("mkdir /tmp/find_test/dir")?;
    nixos_shell.run_cmd("ln -s /tmp/find_test/a.txt /tmp/find_test/symlink")?;

    nixos_shell.run_cmd_and_expect("find /tmp/find_test -name '*.txt'", "a.txt")?;
    nixos_shell.run_cmd_and_expect("find /tmp/find_test -iname '*.txt'", "B.TXT")?;
    nixos_shell.run_cmd_and_expect("find /tmp/find_test -type f", "c.log")?;
    nixos_shell.run_cmd_and_expect("find /tmp/find_test -type d", "dir")?;
    nixos_shell.run_cmd_and_expect("find /tmp/find_test -type l", "symlink")?;

    nixos_shell.run_cmd("find /tmp/find_test -name '*.txt' -delete")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/find_test/a.txt", "No such file or directory")?;
    nixos_shell.run_cmd(r"find /tmp/find_test -name '*.log' -exec cp {} {}.bak \;")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/find_test/c.log.bak", "c.log.bak")?;
    Ok(())
}

#[nixos_test]
fn findutils_xargs(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/xargs_test")?;
    nixos_shell.run_cmd("touch /tmp/xargs_test/a.txt /tmp/xargs_test/b.txt")?;
    nixos_shell.run_cmd("find /tmp/xargs_test -name 'a.txt' | xargs rm")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/xargs_test/a.txt", "No such file or directory")?;
    nixos_shell.run_cmd("find /tmp/xargs_test -name 'b.txt' | xargs -I {} cp {} {}.bak")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/xargs_test/b.txt.bak", "b.txt.bak")?;
    Ok(())
}

// ============================================================================
// Less
// ============================================================================

#[nixos_test]
fn less_viewer(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'less' > /tmp/less_test.txt")?;
    nixos_shell.run_cmd_and_expect("less -F /tmp/less_test.txt", "less")?;
    Ok(())
}
