// SPDX-License-Identifier: MPL-2.0

//! The test suite for development tools on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Compilers - Clang
// ============================================================================

#[nixos_test]
fn clang_compile(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r##"echo -e '#include <stdio.h>\nint main() { printf("Hello from Clang\\n"); return 0; }' > /tmp/hello_clang.c"##)?;
    nixos_shell.run_cmd("clang -o /tmp/hello_clang /tmp/hello_clang.c")?;
    nixos_shell.run_cmd_and_expect("/tmp/hello_clang", "Hello from Clang")?;
    Ok(())
}

#[nixos_test]
fn clang_emit_llvm_ir(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'int main() { return 0; }' > /tmp/simple.c")?;
    nixos_shell.run_cmd("clang -S -emit-llvm /tmp/simple.c -o /tmp/simple.ll")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/simple.ll", "llvm.ident")?;
    Ok(())
}

// ============================================================================
// Compilers - GCC
// ============================================================================

#[nixos_test]
fn gcc_compile(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r##"echo -e '#include <stdio.h>\nint main() { printf("Hello from GCC\\n"); return 0; }' > /tmp/hello.c"##)?;
    nixos_shell.run_cmd("gcc -o /tmp/hello_gcc /tmp/hello.c")?;
    nixos_shell.run_cmd_and_expect("/tmp/hello_gcc", "Hello from GCC")?;
    Ok(())
}

#[nixos_test]
fn gcc_compile_object_file(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'int add(int a, int b) { return a + b; }' > /tmp/add.c")?;
    nixos_shell.run_cmd("gcc -c /tmp/add.c -o /tmp/add.o")?;
    nixos_shell.run_cmd_and_expect("ls -l /tmp/add.o", "add.o")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Go
// ============================================================================

#[nixos_test]
fn go_run_program(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/gotest && cd /tmp/gotest")?;
    nixos_shell.run_cmd(r#"echo -e 'package main\nimport "fmt"\nfunc main() { fmt.Println("Hello from Go") }' > /tmp/gotest/main.go"#)?;
    nixos_shell.run_cmd("cd /tmp/gotest && go mod init gotest")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/gotest && go run main.go", "Hello from Go")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Lua
// ============================================================================

#[nixos_test]
fn lua_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'print("Hello from Lua")' > /tmp/test.lua"#)?;
    nixos_shell.run_cmd_and_expect("lua /tmp/test.lua", "Hello from Lua")?;
    Ok(())
}

#[nixos_test]
fn lua_run_inline_code(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"lua -e "print('Hello World')""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Node.js
// ============================================================================

#[nixos_test]
fn nodejs_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'console.log("Hello from Node.js");' > /tmp/test.js"#)?;
    nixos_shell.run_cmd_and_expect("node /tmp/test.js", "Hello from Node.js")?;
    Ok(())
}

#[nixos_test]
fn nodejs_run_inline_code(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"node -e "console.log('Hello World')""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Octave
// ============================================================================

#[nixos_test]
fn octave_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'disp("Hello from Octave");' > /tmp/octave.m"#)?;
    nixos_shell.run_cmd_and_expect("octave /tmp/octave.m", "Hello from Octave")?;
    Ok(())
}

#[nixos_test]
fn octave_run_inline_code(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"octave --eval "disp('Hello World')""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - OpenJDK
// ============================================================================

#[nixos_test]
fn java_compile_run_program(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'public class HelloWorld { public static void main(String[] args) { System.out.println("Hello from Java"); } }' > /tmp/HelloWorld.java"#)?;
    nixos_shell.run_cmd("cd /tmp && javac HelloWorld.java")?;
    nixos_shell.run_cmd_and_expect("cd /tmp && java HelloWorld", "Hello from Java")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Perl
// ============================================================================

#[nixos_test]
fn perl_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'print "Hello from Perl\n";' > /tmp/test.pl"#)?;
    nixos_shell.run_cmd_and_expect("perl /tmp/test.pl", "Hello from Perl")?;
    Ok(())
}

#[nixos_test]
fn perl_run_inline_code(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"perl -e 'print "Hello World\n"'"#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - PHP
// ============================================================================

#[nixos_test]
fn php_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo '<?php echo "Hello from PHP"; ?>' > /tmp/test.php"#)?;
    nixos_shell.run_cmd_and_expect("php /tmp/test.php", "Hello from PHP")?;
    Ok(())
}

#[nixos_test]
fn php_run_inline_code(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"php -r "echo 'Hello World';""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Python 3
// ============================================================================

#[nixos_test]
fn python3_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'print("Hello from Python")' > /tmp/test.py"#)?;
    nixos_shell.run_cmd_and_expect("python3 /tmp/test.py", "Hello from Python")?;
    Ok(())
}

#[nixos_test]
fn python3_run_inline_code(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"python3 -c "print('Hello World')""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Ruby
// ============================================================================

#[nixos_test]
fn ruby_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'puts "Hello from Ruby"' > /tmp/test.rb"#)?;
    nixos_shell.run_cmd_and_expect("ruby /tmp/test.rb", "Hello from Ruby")?;
    Ok(())
}

#[nixos_test]
fn ruby_run_inline_code(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"ruby -e "puts 'Hello World'""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Rust
// ============================================================================

#[nixos_test]
fn rust_compile_run_program(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rust_test")?;
    nixos_shell
        .run_cmd(r#"echo 'fn main() { println!("Hello from Rust"); }' > /tmp/rust_test/main.rs"#)?;
    nixos_shell.run_cmd("cd /tmp/rust_test && rustc main.rs -o hello_rust")?;
    nixos_shell.run_cmd_and_expect("/tmp/rust_test/hello_rust", "Hello from Rust")?;
    Ok(())
}

// ============================================================================
// Version Control - Git
// ============================================================================

#[nixos_test]
fn git_init_status(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/git_test && git -C /tmp/git_test init -b main")?;
    nixos_shell.run_cmd_and_expect("git -C /tmp/git_test status", "On branch main")?;
    Ok(())
}

#[nixos_test]
fn git_create_commit(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/git_test2 && git -C /tmp/git_test2 init -b main")?;
    nixos_shell.run_cmd("cd /tmp/git_test2 && git config user.email 'test@test.com'")?;
    nixos_shell.run_cmd("cd /tmp/git_test2 && git config user.name 'Test'")?;
    nixos_shell.run_cmd("cd /tmp/git_test2 && echo 'hello' > file.txt")?;
    nixos_shell.run_cmd("cd /tmp/git_test2 && git add file.txt")?;
    nixos_shell.run_cmd_and_expect(
        "cd /tmp/git_test2 && git commit -m 'Initial commit'",
        "Initial commit",
    )?;
    Ok(())
}

#[nixos_test]
fn git_create_branch(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/git_test3 && git -C /tmp/git_test3 init -b main")?;
    nixos_shell.run_cmd("cd /tmp/git_test3 && git config user.email 'test@test.com'")?;
    nixos_shell.run_cmd("cd /tmp/git_test3 && git config user.name 'Test'")?;
    nixos_shell.run_cmd(
        "cd /tmp/git_test3 && echo 'hello' > file.txt && git add . && git commit -m 'init'",
    )?;
    nixos_shell.run_cmd("cd /tmp/git_test3 && git checkout -b feature")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/git_test3 && git branch", "feature")?;
    Ok(())
}

// ============================================================================
// Build Tools - Cargo
// ============================================================================

#[nixos_test]
fn cargo_create_build_project(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("cd /tmp && cargo new myproject")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/myproject && cargo check", "Checking myproject")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/myproject && cargo build", "Finished")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/myproject && cargo run", "Hello, world!")?;
    Ok(())
}

// ============================================================================
// Build Tools - CMake
// ============================================================================

#[nixos_test]
fn cmake_build(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/cmake_test")?;
    nixos_shell.run_cmd(r#"echo -e 'cmake_minimum_required(VERSION 3.0)\nproject(Hello)\nadd_executable(hello main.c)' > /tmp/cmake_test/CMakeLists.txt"#)?;
    nixos_shell.run_cmd(r##"echo -e '#include <stdio.h>\nint main() { printf("Hello from CMake\\n"); return 0; }' > /tmp/cmake_test/main.c"##)?;
    nixos_shell.run_cmd("cd /tmp/cmake_test && mkdir build && cd build")?;
    nixos_shell.run_cmd("cmake -DCMAKE_C_COMPILER=gcc -DCMAKE_CXX_COMPILER=g++ ..")?;
    nixos_shell.run_cmd_and_expect("cmake --build .", "Built target hello")?;
    nixos_shell.run_cmd_and_expect("./hello", "Hello from CMake")?;
    Ok(())
}

// ============================================================================
// Build Tools - Make
// ============================================================================

#[nixos_test]
fn make_run_target(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/make_test")?;
    nixos_shell.run_cmd(r#"echo -e 'all:\n\techo "Hello from Make"' > /tmp/make_test/Makefile"#)?;
    nixos_shell.run_cmd_and_expect("cd /tmp/make_test && make", "Hello from Make")?;
    Ok(())
}

// ============================================================================
// Build Tools - Meson (uses Ninja as the primary backend)
// ============================================================================

#[nixos_test]
fn meson_build(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/meson_test")?;
    nixos_shell.run_cmd(r#"echo -e "project('hello','c')\nexecutable('hello', 'main.c')" > /tmp/meson_test/meson.build"#)?;
    nixos_shell.run_cmd(r##"echo -e '#include <stdio.h>\nint main() { printf("Hello from Meson\\n"); return 0; }' > /tmp/meson_test/main.c"##)?;
    nixos_shell.run_cmd("cd /tmp/meson_test && meson setup builddir")?;
    nixos_shell.run_cmd_and_expect(
        "cd /tmp/meson_test && meson compile -C builddir",
        "Linking target",
    )?;
    nixos_shell.run_cmd_and_expect("/tmp/meson_test/builddir/hello", "Hello from Meson")?;
    Ok(())
}

// ============================================================================
// Debugging Tools - GDB
// ============================================================================

#[nixos_test]
fn gdb_debug(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("gcc -g -O0 /tmp/gdb_sample.c -o /tmp/hello")?;

    nixos_shell.run_cmd_and_expect(
        "gdb -batch -x /tmp/gdb_commands.gdb /tmp/hello > /tmp/gdb.out 2>&1 && echo GDB_OK",
        "GDB_OK",
    )?;

    // Check for expected GDB output.
    nixos_shell.run_cmd_and_expect(
        "grep -F 'Breakpoint 1, hello_world' /tmp/gdb.out",
        "Breakpoint 1, hello_world",
    )?;
    nixos_shell.run_cmd_and_expect("grep -F '#0  hello_world' /tmp/gdb.out", "#0  hello_world")?;
    nixos_shell.run_cmd_and_expect("grep -E '#1 .* in main' /tmp/gdb.out", "in main")?;
    nixos_shell.run_cmd_and_expect("grep -F '$1 = 1' /tmp/gdb.out", "$1 = 1")?;
    nixos_shell.run_cmd_and_expect("grep -F 'rip' /tmp/gdb.out", "rip")?;
    nixos_shell.run_cmd_and_expect("grep -F 'rsp' /tmp/gdb.out", "rsp")?;
    nixos_shell.run_cmd_and_expect(
        "grep -F 'Hello, World 1000!' /tmp/gdb.out",
        "Hello, World 1000!",
    )?;
    nixos_shell.run_cmd_and_expect("grep -F '$2 = (int *)' /tmp/gdb.out", "$2 = (int *)")?;
    nixos_shell.run_cmd_and_expect("grep -F '4321' /tmp/gdb.out", "4321")?;
    nixos_shell.run_cmd_and_expect(
        "grep -F 'memory check passed: 1234' /tmp/gdb.out",
        "memory check passed: 1234",
    )?;
    nixos_shell.run_cmd_and_expect(
        "(! grep -iF 'warning' /tmp/gdb.out) && echo no-warning",
        "no-warning",
    )?;

    Ok(())
}

// ============================================================================
// Debugging Tools - strace
// ============================================================================

#[nixos_test]
fn strace_ls(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(
        "strace -o /tmp/strace.out ls /tmp >/dev/null && echo STRACE_OK",
        "STRACE_OK",
    )?;
    nixos_shell.run_cmd_and_expect("grep -F 'execve(' /tmp/strace.out", "execve(")?;
    nixos_shell.run_cmd_and_expect("grep -F 'getdents64(' /tmp/strace.out", "getdents64(")?;
    Ok(())
}

// ============================================================================
// Hugo
// ============================================================================

#[nixos_test]
fn hugo_serve_site(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("hugo new site /tmp/hugo-test")?;
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "cd /tmp/hugo-test && hugo server -p 4000 > /tmp/hugo.log 2>&1 &",
            CommandCheck::new("curl http://localhost:4000/index.xml", "Hugo"),
            "pkill hugo",
            CommandCheck::new("! pgrep -x hugo >/dev/null && echo stopped", "stopped"),
        ),
        |shell| shell.run_cmd_and_expect("curl http://localhost:4000/index.xml", "Hugo"),
    )?;

    Ok(())
}

// ============================================================================
// direnv
// ============================================================================

#[nixos_test]
fn direnv_load_environment(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/direnv-test")?;
    nixos_shell.run_cmd("export HELLO=hello")?;
    nixos_shell.run_cmd("echo 'export HELLO=world' > /tmp/direnv-test/.envrc")?;

    nixos_shell.run_cmd("cd /tmp/direnv-test && direnv allow")?;
    nixos_shell.run_cmd_and_expect("direnv status", "whitelist")?;
    nixos_shell.run_cmd_and_expect("echo $HELLO", "world")?;
    nixos_shell.run_cmd_and_expect("cd /tmp", "unloading")?;
    nixos_shell.run_cmd_and_expect("echo $HELLO", "hello")?;

    nixos_shell.run_cmd("unset HELLO; rm -rf /tmp/direnv-test")?;
    Ok(())
}

// ============================================================================
// Shellcheck
// ============================================================================

#[nixos_test]
fn shellcheck_lint_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/shellcheck-test")?;
    nixos_shell.run_cmd(r##"echo -e '#!/bin/sh\necho "hello"' > /tmp/shellcheck-test/good.sh"##)?;
    nixos_shell.run_cmd_and_expect(
        "shellcheck /tmp/shellcheck-test/good.sh && echo CLEAN",
        "CLEAN",
    )?;

    nixos_shell.run_cmd(
        r##"echo -e '#!/bin/sh\nname=world\necho "hello $name' > /tmp/shellcheck-test/error.sh"##,
    )?;
    nixos_shell.run_cmd_and_expect("shellcheck /tmp/shellcheck-test/error.sh", "error")?;
    Ok(())
}

// ============================================================================
// jq
// ============================================================================

#[nixos_test]
fn jq_query_json(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"echo '{"name":"Alice","age":30}' | jq ."#, "name")?;
    nixos_shell.run_cmd_and_expect(
        r#"echo '{"user":{"name":"Bob","id":1}}' | jq '.user.name'"#,
        "Bob",
    )?;
    nixos_shell.run_cmd_and_expect(r#"echo '{"a":1,"b":2}' | jq '{sum: (.a + .b)}'"#, "3")?;
    Ok(())
}

// ============================================================================
// yq
// ============================================================================

#[nixos_test]
fn yq_query_yaml(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"echo 'name: Alice' | yq '.name'"#, "Alice")?;
    nixos_shell.run_cmd_and_expect(
        r#"echo -e 'users:\n  - name: Alice\n    age: 30' | yq '.users[].name'"#,
        "Alice",
    )?;
    nixos_shell.run_cmd_and_expect(
        r#"echo -e '- name: Alice\n  age: 35\n- name: Bob\n  age: 25\n' | yq '.[] | select(.age < 30) | .name'"#,
        "Bob",
    )?;
    Ok(())
}
