// SPDX-License-Identifier: MPL-2.0

//! The test suite for networking applications on Asterinas NixOS.
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
// Network Utilities
// ============================================================================

#[nixos_test]
fn curl_basic(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("curl -s https://api.github.com", "current_user_url")?;
    Ok(())
}

#[nixos_test]
fn curl_download(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("curl -s -o /tmp/curl_test.txt https://httpbin.org/robots.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/curl_test.txt", "User-agent")?;
    Ok(())
}

#[nixos_test]
fn wget_download(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("wget -q -O /tmp/wget_test.txt https://httpbin.org/robots.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/wget_test.txt", "User-agent")?;
    Ok(())
}

#[nixos_test]
fn rsync_sync(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rsync_src")?;
    nixos_shell.run_cmd("echo 'rsync test' > /tmp/rsync_src/file.txt")?;
    nixos_shell.run_cmd("mkdir -p /tmp/rsync_dst")?;
    nixos_shell.run_cmd("rsync -av /tmp/rsync_src/ /tmp/rsync_dst/")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/rsync_dst/file.txt", "rsync test")?;
    Ok(())
}

#[nixos_test]
fn rsync_delete(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rsync_src2 /tmp/rsync_dst2")?;
    nixos_shell.run_cmd("echo 'keep' > /tmp/rsync_src2/keep.txt")?;
    nixos_shell.run_cmd("echo 'delete' > /tmp/rsync_dst2/delete.txt")?;
    nixos_shell.run_cmd("rsync -av --delete /tmp/rsync_src2/ /tmp/rsync_dst2/")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/rsync_dst2/delete.txt", "No such file or directory")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/rsync_dst2/keep.txt", "keep")?;
    Ok(())
}

#[nixos_test]
fn rsync_include_exclude(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rsync_src3")?;
    nixos_shell.run_cmd("echo 'txt' > /tmp/rsync_src3/file.txt")?;
    nixos_shell.run_cmd("echo 'log' > /tmp/rsync_src3/file.log")?;
    nixos_shell.run_cmd("echo 'tmp' > /tmp/rsync_src3/file.tmp")?;
    nixos_shell.run_cmd("mkdir -p /tmp/rsync_dst3")?;
    // Include only .txt files, exclude everything else
    nixos_shell
        .run_cmd("rsync -av --include '*.txt' --exclude '*' /tmp/rsync_src3/ /tmp/rsync_dst3/")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/rsync_dst3/file.txt", "file.txt")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/rsync_dst3/file.log", "No such file or directory")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/rsync_dst3/file.tmp", "No such file or directory")?;
    Ok(())
}

#[nixos_test]
fn netcat_listen(nixos_shell: &mut Session) -> Result<(), Error> {
    // Test netcat listening mode
    nixos_shell.run_cmd("echo 'hello from netcat' | nc -l 127.0.0.1 4444 &")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("nc -z 127.0.0.1 4444 && echo 'port open'", "port open")?;
    Ok(())
}

#[nixos_test]
fn lftp_download(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "lftp -c 'open ftp.sjtu.edu.cn; cd /ubuntu-cd; get robots.txt -o /tmp/lftp_test.txt'",
    )?;
    nixos_shell.run_cmd_and_expect("cat /tmp/lftp_test.txt", "User-Agent")?;
    Ok(())
}

#[nixos_test]
fn socat_echo_server(nixos_shell: &mut Session) -> Result<(), Error> {
    // Start a simple echo server in background
    nixos_shell.run_cmd("socat TCP-LISTEN:5555,bind=10.0.2.15,fork EXEC:cat &")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("echo 'test' | socat - TCP:10.0.2.15:5555", "test")?;
    Ok(())
}

#[nixos_test]
fn socat_tcp_connection(nixos_shell: &mut Session) -> Result<(), Error> {
    // Test basic TCP connection
    nixos_shell.run_cmd("socat TCP-LISTEN:6666,bind=10.0.2.15,fork SYSTEM:'echo hello' &")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("socat - TCP:10.0.2.15:6666", "hello")?;
    Ok(())
}

// ============================================================================
// DNS Tools
// ============================================================================

#[nixos_test]
fn ldns_drill_basic(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("drill google.com | head -10", "ANSWER SECTION")?;
    Ok(())
}

#[nixos_test]
fn ldns_drill_record_types(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("drill google.com A | head -20", "ANSWER SECTION")?;
    nixos_shell.run_cmd_and_expect("drill google.com NS | head -20", "ANSWER SECTION")?;
    Ok(())
}

#[nixos_test]
fn ldns_drill_reverse(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("drill -x 8.8.8.8 | head -20", "ANSWER SECTION")?;
    Ok(())
}

#[nixos_test]
fn whois_domain(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("whois google.com | head -20", "Domain Name")?;
    Ok(())
}

#[nixos_test]
fn whois_ip(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("whois 8.8.8.8 | head -20", "Organization")?;
    Ok(())
}
