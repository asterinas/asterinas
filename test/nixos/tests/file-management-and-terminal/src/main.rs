// SPDX-License-Identifier: MPL-2.0

//! The test suite for file management and terminal applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Archiving & Compression - bzip2
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
fn bzip2_cat_grep_archive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'find this' > /tmp/bztest.txt")?;
    nixos_shell.run_cmd("bzip2 /tmp/bztest.txt")?;
    nixos_shell.run_cmd_and_expect("bzcat /tmp/bztest.txt.bz2", "find this")?;
    nixos_shell.run_cmd_and_expect("bzgrep 'find' /tmp/bztest.txt.bz2", "find this")?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - gzip
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
fn gzip_cat_grep_archive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'search me' > /tmp/ztest.txt")?;
    nixos_shell.run_cmd("gzip /tmp/ztest.txt")?;
    nixos_shell.run_cmd_and_expect("zcat /tmp/ztest.txt.gz", "search me")?;
    nixos_shell.run_cmd_and_expect("zgrep 'search' /tmp/ztest.txt.gz", "search me")?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - p7zip
// ============================================================================

#[nixos_test]
fn p7zip_create_extract_archive(nixos_shell: &mut Session) -> Result<(), Error> {
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
fn p7zip_test_archive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/7z_test2")?;
    nixos_shell.run_cmd("echo 'test content' > /tmp/7z_test2/test.txt")?;
    nixos_shell.run_cmd("cd /tmp && 7z a test_archive.7z 7z_test2")?;
    nixos_shell.run_cmd_and_expect("7z t /tmp/test_archive.7z", "Everything is Ok")?;
    Ok(())
}

// ============================================================================
// Archiving & Compression - tar
// ============================================================================

#[nixos_test]
fn tar_create_extract_archive(nixos_shell: &mut Session) -> Result<(), Error> {
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
fn tar_list_archive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/tar_test2")?;
    nixos_shell.run_cmd("echo 'test' > /tmp/tar_test2/test.txt")?;
    nixos_shell.run_cmd("tar -cf /tmp/archive2.tar -C /tmp tar_test2")?;
    nixos_shell.run_cmd_and_expect("tar -tf /tmp/archive2.tar", "tar_test2/test.txt")?;
    Ok(())
}

#[nixos_test]
fn tar_append_file(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/tar_test3")?;
    nixos_shell.run_cmd("echo 'original' > /tmp/tar_test3/original.txt")?;
    nixos_shell.run_cmd("tar -cf /tmp/archive3.tar -C /tmp tar_test3")?;
    nixos_shell.run_cmd("echo 'newfile' > /tmp/newfile.txt")?;
    nixos_shell.run_cmd("tar -rf /tmp/archive3.tar -C /tmp newfile.txt")?;
    nixos_shell.run_cmd_and_expect("tar -tf /tmp/archive3.tar", "newfile.txt")?;
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
fn xz_cat_grep_archive(nixos_shell: &mut Session) -> Result<(), Error> {
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
// Archiving & Compression - Zip
// ============================================================================

#[nixos_test]
fn zip_create_extract_archive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/zip_test")?;
    nixos_shell.run_cmd("cd /tmp/zip_test && echo 'zip content' > zip_file.txt")?;
    nixos_shell.run_cmd("cd /tmp/zip_test && zip archive.zip zip_file.txt")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/zip_test && unzip -l archive.zip", "zip_file.txt")?;
    nixos_shell.run_cmd("cd /tmp/zip_test && rm zip_file.txt")?;
    nixos_shell.run_cmd("cd /tmp/zip_test && unzip archive.zip")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/zip_test && cat zip_file.txt", "zip content")?;
    Ok(())
}

#[nixos_test]
fn zip_list_grep_archive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'search pattern' > /tmp/zip_search.txt")?;
    nixos_shell.run_cmd("zip /tmp/search.zip /tmp/zip_search.txt")?;
    nixos_shell.run_cmd_and_expect("zipinfo /tmp/search.zip", "zip_search.txt")?;
    nixos_shell.run_cmd_and_expect("zipgrep 'pattern' /tmp/search.zip", "search pattern")?;
    Ok(())
}

// ============================================================================
// Terminal Multiplexers - Screen
// ============================================================================

#[nixos_test]
fn screen_manage_session(nixos_shell: &mut Session) -> Result<(), Error> {
    // Start and enter an attached screen
    let screen_desc = SessionDesc::new(
        "screen-test> ",
        "screen -S testsession env PS1='screen-test> ' bash --noprofile --norc -i",
        "exit",
    );
    nixos_shell.enter_session_and_run(screen_desc, |inner| {
        inner.run_cmd_and_expect("screen -ls", "Attached")?;
        Ok(())
    })?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "No Sockets found")?;

    // Start a daemon session in detached mode
    nixos_shell.run_cmd("screen -dmS testsession1")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "Detached")?;
    // Kill the session
    nixos_shell.run_cmd("screen -S testsession1 -X quit")?;
    nixos_shell.run_cmd("sleep 1")?;
    nixos_shell.run_cmd_and_expect("screen -ls", "No Sockets found")?;

    Ok(())
}

// ============================================================================
// File Inspection - file
// ============================================================================

#[nixos_test]
fn file_detect_type(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'text content' > /tmp/textfile.txt")?;
    nixos_shell.run_cmd_and_expect("file /tmp/textfile.txt", "ASCII text")?;
    Ok(())
}

#[nixos_test]
fn file_detect_mime_type(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo '#!/bin/bash' > /tmp/script.sh")?;
    nixos_shell.run_cmd_and_expect("file -i /tmp/script.sh", "text/x-shellscript")?;
    Ok(())
}

// ============================================================================
// Text Processing - bat
// ============================================================================

#[nixos_test]
fn bat_display_file(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'bat test content' > /tmp/bat_test.txt")?;
    nixos_shell.run_cmd_and_expect("bat /tmp/bat_test.txt", "bat test content")?;
    Ok(())
}

#[nixos_test]
fn bat_display_plain_output(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'plain output' > /tmp/bat_plain.txt")?;
    nixos_shell.run_cmd_and_expect("bat --plain /tmp/bat_plain.txt", "plain output")?;
    Ok(())
}

// ============================================================================
// Text Processing - gawk
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
fn gawk_sum_numbers(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e '10\n20\n30' > /tmp/numbers.txt")?;
    nixos_shell.run_cmd_and_expect("awk '{sum += $1} END {print sum}' /tmp/numbers.txt", "60")?;
    Ok(())
}

// ============================================================================
// Text Processing - sd
// ============================================================================

#[nixos_test]
fn sd_replace_in_files(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/sd_test")?;
    nixos_shell.run_cmd("echo 'hello' > /tmp/sd_test/hello.txt")?;
    nixos_shell.run_cmd(r#"cd /tmp/sd_test && sd "hello" "world" hello.txt"#)?;
    nixos_shell.run_cmd_and_expect("cat /tmp/sd_test/hello.txt", "world")?;
    Ok(())
}

// ============================================================================
// Text Processing - sed
// ============================================================================

#[nixos_test]
fn sed_print_lines(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e 'line1\nline2\nline3\nline4\nline5' > /tmp/sed_test.txt")?;
    nixos_shell.run_cmd_and_expect("sed -n '2,4p' /tmp/sed_test.txt", "line2")?;
    Ok(())
}

#[nixos_test]
fn sed_replace_text(nixos_shell: &mut Session) -> Result<(), Error> {
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
// Search & Filtering - eza
// ============================================================================

#[nixos_test]
fn eza_list_files(nixos_shell: &mut Session) -> Result<(), Error> {
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

// ============================================================================
// Search & Filtering - fd
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
fn fd_execute_command(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/fd_exec")?;
    nixos_shell.run_cmd("touch /tmp/fd_exec/to_delete.txt")?;
    nixos_shell.run_cmd("fd 'to_delete' /tmp/fd_exec -x rm")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/fd_exec/to_delete.txt", "No such file or directory")?;
    Ok(())
}

// ============================================================================
// Search & Filtering - fzf
// ============================================================================

#[nixos_test]
fn fzf_filter_candidates(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r"echo -e 'apple\nbanana\ncherry\napricot' > /tmp/fzf_test.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/fzf_test.txt | fzf -f 'ap'", "apple")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/fzf_test.txt | fzf -f 'ap'", "apricot")?;
    Ok(())
}

// ============================================================================
// Search & Filtering - ripgrep
// ============================================================================

#[nixos_test]
fn ripgrep_search_pattern(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rg_test")?;
    nixos_shell.run_cmd("echo 'find this pattern' > /tmp/rg_test/file1.txt")?;
    nixos_shell.run_cmd("echo 'different content' > /tmp/rg_test/file2.txt")?;
    nixos_shell.run_cmd_and_expect("rg 'pattern' /tmp/rg_test", "pattern")?;
    Ok(())
}

#[nixos_test]
fn ripgrep_search_case_insensitive(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'UPPERCASE' > /tmp/rg_case.txt")?;
    nixos_shell.run_cmd_and_expect("rg -i 'uppercase' /tmp/rg_case.txt", "UPPERCASE")?;
    Ok(())
}

#[nixos_test]
fn ripgrep_search_literal_text(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'test.*pattern' > /tmp/rg_literal.txt")?;
    nixos_shell.run_cmd_and_expect("rg -F '.*' /tmp/rg_literal.txt", "test.*pattern")?;
    Ok(())
}

#[nixos_test]
fn ripgrep_list_matching_files(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rg_names")?;
    nixos_shell.run_cmd("echo 'unique content' > /tmp/rg_names/match.txt")?;
    nixos_shell.run_cmd("echo 'other' > /tmp/rg_names/nomatch.txt")?;
    nixos_shell.run_cmd_and_expect("rg -l 'unique' /tmp/rg_names", "match.txt")?;
    Ok(())
}

// ============================================================================
// Search & Filtering - The Silver Searcher
// ============================================================================

#[nixos_test]
fn silver_searcher_pattern_search(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/ag_test")?;
    nixos_shell.run_cmd(r#"echo 'find this pattern' > /tmp/ag_test/notes.txt"#)?;
    nixos_shell.run_cmd_and_expect(r#"ag "pattern" /tmp/ag_test"#, "find this pattern")?;
    Ok(())
}

#[nixos_test]
fn silver_searcher_cc_filter(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/ag_cc")?;
    nixos_shell.run_cmd(r#"echo 'int main(void) { return 0; }' > /tmp/ag_cc/main.c"#)?;
    nixos_shell.run_cmd(r#"echo 'main should not match here' > /tmp/ag_cc/main.txt"#)?;
    nixos_shell.run_cmd_and_expect(r#"ag --cc "main" /tmp/ag_cc"#, "main.c")?;
    Ok(())
}

// ============================================================================
// Search & Filtering - tree
// ============================================================================

#[nixos_test]
fn tree_list_depth_limited(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/tree_test/level1/level2")?;
    nixos_shell.run_cmd("touch /tmp/tree_test/level1/file.txt")?;
    nixos_shell.run_cmd_and_expect("tree -L 2 /tmp/tree_test", "level2")?;
    Ok(())
}

#[nixos_test]
fn tree_show_hidden_files(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/tree_hidden")?;
    nixos_shell.run_cmd("touch /tmp/tree_hidden/.secret /tmp/tree_hidden/visible")?;
    nixos_shell.run_cmd_and_expect("tree -a /tmp/tree_hidden", ".secret")?;
    Ok(())
}

// ============================================================================
// Security & Backup - age
// ============================================================================

#[nixos_test]
fn age_encrypt_decrypt_file(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'top secret' > /tmp/age-secret.txt")?;
    nixos_shell.run_cmd_and_expect("age-keygen -o /tmp/age-key.txt", "Public key")?;
    nixos_shell.run_cmd(
        r#"age -r "$(sed -n 's/^# public key: //p' /tmp/age-key.txt)" -o /tmp/age-secret.txt.age /tmp/age-secret.txt"#,
    )?;
    nixos_shell
        .run_cmd("age -d -i /tmp/age-key.txt -o /tmp/age-decrypted.txt /tmp/age-secret.txt.age")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/age-decrypted.txt", "top secret")?;
    Ok(())
}

// ============================================================================
// Security & Backup - crunch
// ============================================================================

#[nixos_test]
fn crunch_generate_wordlist(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("crunch 2 2 ab -o /tmp/crunch-wordlist.txt")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/crunch-wordlist.txt", "aa")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/crunch-wordlist.txt", "bb")?;
    Ok(())
}

#[nixos_test]
fn crunch_generate_pattern_list(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("crunch 4 4 -t @@%% -o /tmp/crunch-pattern.txt")?;
    nixos_shell.run_cmd_and_expect(
        r#"grep -E "^[a-z]{2}[0-9]{2}$" /tmp/crunch-pattern.txt | head -1"#,
        "aa00",
    )?;
    Ok(())
}

// ============================================================================
// Security & Backup - GnuPG
// ============================================================================

#[nixos_test]
fn gnupg_generate_export_sign_and_verify(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/gnupg-test")?;
    nixos_shell.run_cmd(
        r#"gpg --batch --homedir /tmp/gnupg-test --pinentry-mode loopback --passphrase-fd 0 --quick-generate-key "Test User <test@example.com>" ed25519 sign never <<< """#,
    )?;
    nixos_shell.run_cmd_and_expect(
        "gpg --homedir /tmp/gnupg-test --list-keys",
        "test@example.com",
    )?;
    nixos_shell.run_cmd(
        "gpg --homedir /tmp/gnupg-test --export --armor test@example.com > /tmp/public_key.asc",
    )?;
    nixos_shell.run_cmd(
        "gpg --homedir /tmp/gnupg-test --export-secret-keys --armor test@example.com > /tmp/private_key.asc",
    )?;
    nixos_shell.run_cmd_and_expect("cat /tmp/public_key.asc", "BEGIN PGP PUBLIC KEY BLOCK")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/private_key.asc", "BEGIN PGP PRIVATE KEY BLOCK")?;
    nixos_shell.run_cmd("echo 'signed content' > /tmp/gpg-file.txt")?;
    nixos_shell.run_cmd(
        "gpg --batch --yes --homedir /tmp/gnupg-test --local-user test@example.com --output /tmp/gpg-file.txt.gpg --sign /tmp/gpg-file.txt",
    )?;
    nixos_shell.run_cmd_and_expect(
        "gpg --homedir /tmp/gnupg-test --verify /tmp/gpg-file.txt.gpg 2>&1",
        "Good signature",
    )?;
    nixos_shell.run_cmd(
        "gpg --batch --yes --homedir /tmp/gnupg-test --output /tmp/gpg-verified.txt /tmp/gpg-file.txt.gpg",
    )?;
    nixos_shell.run_cmd_and_expect("cat /tmp/gpg-verified.txt", "signed content")?;
    Ok(())
}

#[nixos_test]
fn gnupg_symmetric_encrypt_decrypt(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/gnupg-test")?;
    nixos_shell.run_cmd("echo 'gpg secret' > /tmp/gpg-secret.txt")?;
    nixos_shell.run_cmd(
        r#"gpg --symmetric --passphrase "testpassword" --output /tmp/gpg-encrypted.txt --batch /tmp/gpg-secret.txt"#,
    )?;
    nixos_shell.run_cmd(
        r#"gpg --decrypt --passphrase "testpassword" --batch /tmp/gpg-encrypted.txt > /tmp/gpg-decrypted.txt"#,
    )?;
    nixos_shell.run_cmd_and_expect("cat /tmp/gpg-decrypted.txt", "gpg secret")?;
    Ok(())
}

// ============================================================================
// Security & Backup - John the Ripper
// ============================================================================

#[nixos_test]
fn john_crack_password_hash(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        r#"echo -e 'password\n123456\nadmin\nwelcome\nqwerty\nabc123\npassword123' > /tmp/john-wordlist.txt"#,
    )?;
    nixos_shell.run_cmd("echo '482c811da5d5b4bc6d497ffa98491e38' > /tmp/john-hashes.txt")?;
    nixos_shell.run_cmd(
        "john --format=raw-md5 --pot=/tmp/john.pot --wordlist=/tmp/john-wordlist.txt /tmp/john-hashes.txt",
    )?;
    nixos_shell.run_cmd_and_expect(
        "john --format=raw-md5 --pot=/tmp/john.pot --show /tmp/john-hashes.txt",
        "password123",
    )?;
    Ok(())
}

// ============================================================================
// Security & Backup - restic
// ============================================================================

#[nixos_test]
fn restic_backup_list_snapshots(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/restic-src")?;
    nixos_shell.run_cmd("echo 'backup data' > /tmp/restic-src/file.txt")?;
    nixos_shell.run_cmd("RESTIC_PASSWORD=testpass restic -r /tmp/restic-repo init")?;
    nixos_shell
        .run_cmd("RESTIC_PASSWORD=testpass restic -r /tmp/restic-repo backup /tmp/restic-src")?;
    nixos_shell.run_cmd_and_expect(
        "RESTIC_PASSWORD=testpass restic -r /tmp/restic-repo snapshots",
        "/tmp/restic-src",
    )?;
    Ok(())
}

// ============================================================================
// Security & Backup - wipe
// ============================================================================

#[nixos_test]
fn wipe_overwrite_file_zero_pass(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'erase me' > /tmp/wipe-zero.txt")?;
    nixos_shell.run_cmd("wipe -f -z /tmp/wipe-zero.txt")?;
    nixos_shell.run_cmd_and_expect("test ! -e /tmp/wipe-zero.txt && echo deleted", "deleted")?;
    Ok(())
}

#[nixos_test]
fn wipe_overwrite_file_multiple_passes(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'erase again' > /tmp/wipe-passes.txt")?;
    nixos_shell.run_cmd("wipe -f -p 8 /tmp/wipe-passes.txt")?;
    nixos_shell.run_cmd_and_expect("test ! -e /tmp/wipe-passes.txt && echo deleted", "deleted")?;
    Ok(())
}
