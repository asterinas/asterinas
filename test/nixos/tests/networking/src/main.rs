// SPDX-License-Identifier: MPL-2.0

//! The test suite for networking applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Network Utilities
// ============================================================================

#[nixos_test]
fn curl_fetch_webpage(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("curl -sL https://bing.com", "doctype html")?;
    Ok(())
}

#[nixos_test]
fn curl_download(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("curl -sL -o /tmp/curl_test.txt https://bing.com/robots.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/curl_test.txt", "Disallow")?;
    Ok(())
}

#[nixos_test]
fn lftp_download(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("lftp -c 'open ftp.gnu.org; cd /; get README -o /tmp/lftp_test.txt'")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/lftp_test.txt", "GNU project")?;
    Ok(())
}

#[nixos_test]
fn netcat_accept_connection(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "nc -k -l 127.0.0.1 4444 &",
            CommandCheck::new("nc -z -w 1 127.0.0.1 4444", "succeeded"),
            "pkill -f 'nc -k -l 127.0.0.1 4444' || true",
            CommandCheck::new(
                "! pgrep -f 'nc -k -l 127.0.0.1 4444' >/dev/null && echo stopped",
                "stopped",
            ),
        ),
        |shell| shell.run_cmd_and_expect("nc -w 1 127.0.0.1 4444 && echo ok", "ok"),
    )
}

#[nixos_test]
fn rclone_sync_files(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rclone-src/subdir")?;
    nixos_shell.run_cmd("echo 'hello' > /tmp/rclone-src/a.txt")?;
    nixos_shell.run_cmd("echo 'world' > /tmp/rclone-src/b.txt")?;
    nixos_shell.run_cmd("echo 'nested' > /tmp/rclone-src/subdir/c.txt")?;
    nixos_shell.run_cmd("mkdir -p /tmp/rclone-dst")?;

    nixos_shell.run_cmd("rclone copy /tmp/rclone-src /tmp/rclone-dst")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/rclone-dst/a.txt", "hello")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/rclone-dst/b.txt", "world")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/rclone-dst/subdir/c.txt", "nested")?;

    nixos_shell.run_cmd("echo 'extra' > /tmp/rclone-dst/extra.txt")?;
    nixos_shell.run_cmd("rclone sync /tmp/rclone-src /tmp/rclone-dst")?;
    nixos_shell.run_cmd_and_expect(
        "test -f /tmp/rclone-dst/extra.txt && echo 'exists' || echo 'deleted'",
        "deleted",
    )?;

    nixos_shell.run_cmd_and_expect(
        "rclone check /tmp/rclone-src /tmp/rclone-dst",
        "matching files",
    )?;
    nixos_shell.run_cmd_and_expect("rclone size /tmp/rclone-src", "Total objects")?;

    nixos_shell.run_cmd_and_expect("rclone lsl /tmp/rclone-src", "a.txt")?;
    nixos_shell.run_cmd_and_expect("rclone lsd /tmp/rclone-src", "subdir")?;
    nixos_shell.run_cmd("rclone mkdir /tmp/rclone-dst/newdir")?;
    nixos_shell.run_cmd_and_expect("test -d /tmp/rclone-dst/newdir && echo 'ok'", "ok")?;
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
fn socat_proxy_echo(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "socat TCP-LISTEN:5555,bind=10.0.2.15,fork EXEC:cat &",
            CommandCheck::new(
                "socat /dev/null TCP:10.0.2.15:5555,connect-timeout=1 && echo ok",
                "ok",
            ),
            "pkill -f TCP-LISTEN:5555 || true",
            CommandCheck::new(
                "! pgrep -f TCP-LISTEN:5555 >/dev/null && echo stopped",
                "stopped",
            ),
        ),
        |shell| shell.run_cmd_and_expect("echo 'hello' | socat - TCP:10.0.2.15:5555", "hello"),
    )
}

#[nixos_test]
fn socat_serve_tcp_response(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "socat TCP-LISTEN:6666,bind=10.0.2.15,fork SYSTEM:'echo hello' &",
            CommandCheck::new(
                "socat /dev/null TCP:10.0.2.15:6666,connect-timeout=1 && echo ok",
                "ok",
            ),
            "pkill -f TCP-LISTEN:6666 || true",
            CommandCheck::new(
                "! pgrep -f TCP-LISTEN:6666 >/dev/null && echo stopped",
                "stopped",
            ),
        ),
        |shell| shell.run_cmd_and_expect("socat - TCP:10.0.2.15:6666", "hello"),
    )
}

#[nixos_test]
fn wget_download(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("wget -q -O /tmp/wget_test.txt https://bing.com/robots.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/wget_test.txt", "Disallow")?;
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
fn whois_lookup_domain(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("whois google.com | head -20", "Domain Name")?;
    Ok(())
}

#[nixos_test]
fn whois_lookup_ip(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("whois 8.8.8.8 | head -20", "Organization")?;
    Ok(())
}
