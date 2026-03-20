// SPDX-License-Identifier: MPL-2.0

//! The test suite for file management and terminal productivity applications on Asterinas NixOS.
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
// Archiving & Compression - GNU tar
// ============================================================================

#[nixos_test]
fn tar_create_extract(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/tar_test")?;
    nixos_shell.run_cmd("echo 'file1 content' > /tmp/tar_test/file1.txt")?;
    nixos_shell.run_cmd("echo 'file2 content' > /tmp/tar_test/file2.txt")?;
    nixos_shell.run_cmd("tar -cf /tmp/archive.tar -C /tmp tar_test")?;
    nixos_shell.run_cmd("rm -rf /tmp/tar_test")?;
    nixos_shell.run_cmd("tar -xf /tmp/archive.tar -C /tmp")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/tar_test/file1.txt", "file1 content")?;
    Ok(())
}

#[nixos_test]
fn tar_list(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/tar_test2")?;
    nixos_shell.run_cmd("echo 'test' > /tmp/tar_test2/test.txt")?;
    nixos_shell.run_cmd("tar -cf /tmp/archive2.tar -C /tmp tar_test2")?;
    nixos_shell.run_cmd_and_expect("tar -tf /tmp/archive2.tar", "tar_test2/test.txt")?;
    Ok(())
}

#[nixos_test]
fn tar_append(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/tar_test3")?;
    nixos_shell.run_cmd("echo 'original' > /tmp/tar_test3/original.txt")?;
    nixos_shell.run_cmd("tar -cf /tmp/archive3.tar -C /tmp tar_test3")?;
    nixos_shell.run_cmd("echo 'newfile' > /tmp/newfile.txt")?;
    nixos_shell.run_cmd("tar -rf /tmp/archive3.tar -C /tmp newfile.txt")?;
    nixos_shell.run_cmd_and_expect("tar -tf /tmp/archive3.tar", "newfile.txt")?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - Gzip
// ============================================================================

#[nixos_test]
fn gzip_compress_decompress(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'hello gzip' > /tmp/gzip_test.txt")?;
    nixos_shell.run_cmd("gzip /tmp/gzip_test.txt")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/gzip_test.txt.gz", "gzip_test.txt.gz")?;
    nixos_shell.run_cmd("gunzip /tmp/gzip_test.txt.gz")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/gzip_test.txt", "hello gzip")?;
    Ok(())
}

#[nixos_test]
fn gzip_zcat_zgrep(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'search me' > /tmp/ztest.txt")?;
    nixos_shell.run_cmd("gzip /tmp/ztest.txt")?;
    nixos_shell.run_cmd_and_expect("zcat /tmp/ztest.txt.gz", "search me")?;
    nixos_shell.run_cmd_and_expect("zgrep 'search' /tmp/ztest.txt.gz", "search me")?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - Bzip2
// ============================================================================

#[nixos_test]
fn bzip2_compress_decompress(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'hello bzip2' > /tmp/bzip2_test.txt")?;
    nixos_shell.run_cmd("bzip2 /tmp/bzip2_test.txt")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/bzip2_test.txt.bz2", "bzip2_test.txt.bz2")?;
    nixos_shell.run_cmd("bunzip2 /tmp/bzip2_test.txt.bz2")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/bzip2_test.txt", "hello bzip2")?;
    Ok(())
}

#[nixos_test]
fn bzip2_bzcat_bzgrep(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'find this' > /tmp/bztest.txt")?;
    nixos_shell.run_cmd("bzip2 /tmp/bztest.txt")?;
    nixos_shell.run_cmd_and_expect("bzcat /tmp/bztest.txt.bz2", "find this")?;
    nixos_shell.run_cmd_and_expect("bzgrep 'find' /tmp/bztest.txt.bz2", "find this")?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - XZ Utils
// ============================================================================

#[nixos_test]
fn xz_compress_decompress(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'hello xz' > /tmp/xz_test.txt")?;
    nixos_shell.run_cmd("xz /tmp/xz_test.txt")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/xz_test.txt.xz", "xz_test.txt.xz")?;
    nixos_shell.run_cmd("unxz /tmp/xz_test.txt.xz")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/xz_test.txt", "hello xz")?;
    Ok(())
}

#[nixos_test]
fn xz_xzcat_xzgrep(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'xz content here' > /tmp/xzgrep_test.txt")?;
    nixos_shell.run_cmd("xz /tmp/xzgrep_test.txt")?;
    nixos_shell.run_cmd_and_expect("xzcat /tmp/xzgrep_test.txt.xz", "xz content here")?;
    nixos_shell.run_cmd_and_expect(
        "xzgrep 'content' /tmp/xzgrep_test.txt.xz",
        "xz content here",
    )?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - p7zip
// ============================================================================

#[nixos_test]
fn p7zip_create_extract(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/7z_test")?;
    nixos_shell.run_cmd("echo 'seven zip' > /tmp/7z_test/file.txt")?;
    nixos_shell.run_cmd("cd /tmp && 7z a archive.7z 7z_test")?;
    nixos_shell.run_cmd_and_expect("7z l /tmp/archive.7z", "file.txt")?;
    nixos_shell.run_cmd("rm -rf /tmp/7z_test")?;
    nixos_shell.run_cmd("7z x /tmp/archive.7z -o/tmp/extracted")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/extracted/7z_test/file.txt", "seven zip")?;
    Ok(())
}

#[nixos_test]
fn p7zip_test(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/7z_test2")?;
    nixos_shell.run_cmd("echo 'test content' > /tmp/7z_test2/test.txt")?;
    nixos_shell.run_cmd("cd /tmp && 7z a test_archive.7z 7z_test2")?;
    nixos_shell.run_cmd_and_expect("7z t /tmp/test_archive.7z", "Everything is Ok")?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - Zip
// ============================================================================

#[nixos_test]
fn zip_create_extract(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'zip content' > /tmp/zip_file.txt")?;
    nixos_shell.run_cmd("zip /tmp/archive.zip /tmp/zip_file.txt")?;
    nixos_shell.run_cmd_and_expect("unzip -l /tmp/archive.zip", "zip_file.txt")?;
    nixos_shell.run_cmd("rm /tmp/zip_file.txt")?;
    nixos_shell.run_cmd("cd / && unzip /tmp/archive.zip")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/zip_file.txt", "zip content")?;
    Ok(())
}

#[nixos_test]
fn zip_zipinfo_zipgrep(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'search pattern' > /tmp/zip_search.txt")?;
    nixos_shell.run_cmd("zip /tmp/search.zip /tmp/zip_search.txt")?;
    nixos_shell.run_cmd_and_expect("zipinfo /tmp/search.zip", "zip_search.txt")?;
    nixos_shell.run_cmd_and_expect("zipgrep 'pattern' /tmp/search.zip", "search pattern")?;
    Ok(())
}

// ============================================================================
// Terminal Multiplexers - GNU Screen
// ============================================================================

#[nixos_test]
fn screen_session(nixos_shell: &mut Session) -> Result<(), Error> {
    // Start a session
    nixos_shell.run_cmd("screen -S testsession")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "Attached")?;
    // Exit the session
    nixos_shell.run_cmd_and_expect("exit", "screen is terminating")?;

    // Start a daemon session in detached mode
    nixos_shell.run_cmd("screen -dmS testsession1")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "Detached")?;
    // Reattach to a detached session
    nixos_shell.run_cmd("screen -r testsession1")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "Attached")?;
    // Exit the session
    nixos_shell.run_cmd_and_expect("exit", "screen is terminating")?;

    // Start a daemon session in detached mode
    nixos_shell.run_cmd("screen -dmS testsession2")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "testsession2")?;
    // Kill the session
    nixos_shell.run_cmd("screen -S testsession2 -X quit")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "No Sockets found")?;

    Ok(())
}

// ============================================================================
// Modern CLI Utilities - File
// ============================================================================

#[nixos_test]
fn file_type_detection(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'text content' > /tmp/textfile.txt")?;
    nixos_shell.run_cmd_and_expect("file /tmp/textfile.txt", "ASCII text")?;
    Ok(())
}

#[nixos_test]
fn file_mime_type(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo '#!/bin/bash' > /tmp/script.sh")?;
    nixos_shell.run_cmd_and_expect("file -i /tmp/script.sh", "text/x-shellscript")?;
    Ok(())
}

// ============================================================================
// Modern CLI Utilities - Gawk
// ============================================================================

#[nixos_test]
fn gawk_field_separator(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'a:b:c' > /tmp/awk_test.txt")?;
    nixos_shell.run_cmd_and_expect("awk -F: '{print $2}' /tmp/awk_test.txt", "b")?;
    Ok(())
}

#[nixos_test]
fn gawk_pattern_match(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e 'line1\npattern line\nline3' > /tmp/awk_pattern.txt")?;
    nixos_shell.run_cmd_and_expect(
        "awk '/pattern/ {print}' /tmp/awk_pattern.txt",
        "pattern line",
    )?;
    Ok(())
}

#[nixos_test]
fn gawk_sum(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e '10\n20\n30' > /tmp/numbers.txt")?;
    nixos_shell.run_cmd_and_expect("awk '{sum += $1} END {print sum}' /tmp/numbers.txt", "60")?;
    Ok(())
}

// ============================================================================
// Modern CLI Utilities - GNU sed
// ============================================================================

#[nixos_test]
fn sed_print_lines(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e 'line1\nline2\nline3\nline4\nline5' > /tmp/sed_test.txt")?;
    nixos_shell.run_cmd_and_expect("sed -n '2,4p' /tmp/sed_test.txt", "line2")?;
    Ok(())
}

#[nixos_test]
fn sed_replace(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'hello world world' > /tmp/sed_replace.txt")?;
    nixos_shell.run_cmd_and_expect(
        "sed 's/world/replaced/g' /tmp/sed_replace.txt",
        "hello replaced replaced",
    )?;
    Ok(())
}

#[nixos_test]
fn sed_delete_lines(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e 'line1\nline2\nline3' > /tmp/sed_delete.txt")?;
    nixos_shell.run_cmd_and_expect("sed '2d' /tmp/sed_delete.txt", "line1")?;
    nixos_shell.run_cmd_and_expect("sed '2d' /tmp/sed_delete.txt", "line3")?;
    Ok(())
}

// ============================================================================
// Modern CLI Utilities - fzf
// ============================================================================

#[nixos_test]
fn fzf_filter(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e 'apple\nbanana\ncherry\napricot' > /tmp/fzf_test.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/fzf_test.txt | fzf -f 'ap'", "apple")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/fzf_test.txt | fzf -f 'ap'", "apricot")?;
    Ok(())
}

// ============================================================================
// Modern CLI Utilities - ripgrep
// ============================================================================

#[nixos_test]
fn ripgrep_search(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rg_test")?;
    nixos_shell.run_cmd("echo 'find this pattern' > /tmp/rg_test/file1.txt")?;
    nixos_shell.run_cmd("echo 'different content' > /tmp/rg_test/file2.txt")?;
    nixos_shell.run_cmd_and_expect("rg 'pattern' /tmp/rg_test", "pattern")?;
    Ok(())
}

#[nixos_test]
fn ripgrep_case_insensitive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'UPPERCASE' > /tmp/rg_case.txt")?;
    nixos_shell.run_cmd_and_expect("rg -i 'uppercase' /tmp/rg_case.txt", "UPPERCASE")?;
    Ok(())
}

#[nixos_test]
fn ripgrep_literal(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'test.*pattern' > /tmp/rg_literal.txt")?;
    nixos_shell.run_cmd_and_expect("rg -F '.*' /tmp/rg_literal.txt", "test.*pattern")?;
    Ok(())
}

#[nixos_test]
fn ripgrep_filenames(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rg_names")?;
    nixos_shell.run_cmd("echo 'unique content' > /tmp/rg_names/match.txt")?;
    nixos_shell.run_cmd("echo 'other' > /tmp/rg_names/nomatch.txt")?;
    nixos_shell.run_cmd_and_expect("rg -l 'unique' /tmp/rg_names", "match.txt")?;
    Ok(())
}

// ============================================================================
// Modern CLI Utilities - fd
// ============================================================================

#[nixos_test]
fn fd_find_files(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/fd_test")?;
    nixos_shell
        .run_cmd("touch /tmp/fd_test/file1.txt /tmp/fd_test/file2.txt /tmp/fd_test/other.log")?;
    nixos_shell.run_cmd_and_expect("fd '.txt' /tmp/fd_test", "file1.txt")?;
    nixos_shell.run_cmd_and_expect("fd '.txt' /tmp/fd_test", "file2.txt")?;
    Ok(())
}

#[nixos_test]
fn fd_execute(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/fd_exec")?;
    nixos_shell.run_cmd("touch /tmp/fd_exec/to_delete.txt")?;
    nixos_shell.run_cmd("fd 'to_delete' /tmp/fd_exec -x rm")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/fd_exec/to_delete.txt", "No such file or directory")?;
    Ok(())
}

// ============================================================================
// Modern CLI Utilities - bat
// ============================================================================

#[nixos_test]
fn bat_display(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'bat test content' > /tmp/bat_test.txt")?;
    nixos_shell.run_cmd_and_expect("bat /tmp/bat_test.txt", "bat test content")?;
    Ok(())
}

#[nixos_test]
fn bat_plain(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'plain output' > /tmp/bat_plain.txt")?;
    nixos_shell.run_cmd_and_expect("bat --plain /tmp/bat_plain.txt", "plain output")?;
    Ok(())
}

// ============================================================================
// Modern CLI Utilities - eza
// ============================================================================

#[nixos_test]
fn eza_list(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/eza_test")?;
    nixos_shell.run_cmd("touch /tmp/eza_test/file1 /tmp/eza_test/file2")?;
    nixos_shell.run_cmd_and_expect("eza /tmp/eza_test", "file1")?;
    nixos_shell.run_cmd_and_expect("eza /tmp/eza_test", "file2")?;
    Ok(())
}

#[nixos_test]
fn eza_long_hidden(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/eza_long")?;
    nixos_shell.run_cmd("touch /tmp/eza_long/.hidden /tmp/eza_long/visible")?;
    nixos_shell.run_cmd_and_expect("eza -la /tmp/eza_long", ".hidden")?;
    nixos_shell.run_cmd_and_expect("eza -la /tmp/eza_long", "visible")?;
    Ok(())
}

#[nixos_test]
fn eza_directories_only(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/eza_dir/subdir")?;
    nixos_shell.run_cmd("touch /tmp/eza_dir/file.txt")?;
    nixos_shell.run_cmd_and_expect("eza -D /tmp/eza_dir", "subdir")?;
    Ok(())
}
