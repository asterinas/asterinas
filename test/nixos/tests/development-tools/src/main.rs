// SPDX-License-Identifier: MPL-2.0

//! The test suite for development tools applications on Asterinas NixOS.
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
// Programming Language Runtimes - Python3
// ============================================================================

#[nixos_test]
fn python3_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'print("Hello from Python")' > /tmp/test.py"#)?;
    nixos_shell.run_cmd_and_expect("python3 /tmp/test.py", "Hello from Python")?;
    Ok(())
}

#[nixos_test]
fn python3_inline(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"python3 -c "print('Hello World')""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Node.js
// ============================================================================

#[nixos_test]
fn nodejs_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'console.log("Hello from Node.js");' > /tmp/test.js"#)?;
    nixos_shell.run_cmd_and_expect("node /tmp/test.js", "Hello from Node.js")?;
    Ok(())
}

#[nixos_test]
fn nodejs_inline(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"node -e "console.log('Hello World')""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Go
// ============================================================================

#[nixos_test]
fn go_run(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/gotest && cd /tmp/gotest")?;
    nixos_shell.run_cmd(r#"echo -e 'package main\nimport "fmt"\nfunc main() { fmt.Println("Hello from Go") }' > /tmp/gotest/main.go"#)?;
    nixos_shell.run_cmd("cd /tmp/gotest && go mod init gotest")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/gotest && go run main.go", "Hello from Go")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Rust
// ============================================================================

#[nixos_test]
fn rust_compile_run(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/rust_test")?;
    nixos_shell
        .run_cmd(r#"echo 'fn main() { println!("Hello from Rust"); }' > /tmp/rust_test/main.rs"#)?;
    nixos_shell.run_cmd("cd /tmp/rust_test && rustc main.rs -o hello_rust")?;
    nixos_shell.run_cmd_and_expect("/tmp/rust_test/hello_rust", "Hello from Rust")?;
    Ok(())
}

#[nixos_test]
fn cargo_new_build(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("cd /tmp && cargo new myproject")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/myproject && cargo check", "Checking myproject")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/myproject && cargo build", "Compiling myproject")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/myproject && cargo run", "Hello, world!")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - OpenJDK
// ============================================================================

#[nixos_test]
fn java_compile_run(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'public class HelloWorld { public static void main(String[] args) { System.out.println("Hello from Java"); } }' > /tmp/HelloWorld.java"#)?;
    nixos_shell.run_cmd("cd /tmp && javac HelloWorld.java")?;
    nixos_shell.run_cmd_and_expect("cd /tmp && java HelloWorld", "Hello from Java")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Ruby
// ============================================================================

#[nixos_test]
fn ruby_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'puts "Hello from Ruby"' > /tmp/test.rb"#)?;
    nixos_shell.run_cmd_and_expect("ruby /tmp/test.rb", "Hello from Ruby")?;
    Ok(())
}

#[nixos_test]
fn ruby_inline(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"ruby -e "puts 'Hello World'""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Perl
// ============================================================================

#[nixos_test]
fn perl_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'print "Hello from Perl\n";' > /tmp/test.pl"#)?;
    nixos_shell.run_cmd_and_expect("perl /tmp/test.pl", "Hello from Perl")?;
    Ok(())
}

#[nixos_test]
fn perl_inline(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"perl -e 'print "Hello World\n"'"#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - Lua
// ============================================================================

#[nixos_test]
fn lua_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo 'print("Hello from Lua")' > /tmp/test.lua"#)?;
    nixos_shell.run_cmd_and_expect("lua /tmp/test.lua", "Hello from Lua")?;
    Ok(())
}

#[nixos_test]
fn lua_inline(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"lua -e "print('Hello World')""#, "Hello World")?;
    Ok(())
}

// ============================================================================
// Programming Language Runtimes - PHP
// ============================================================================

#[nixos_test]
fn php_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r#"echo '<?php echo "Hello from PHP"; ?>' > /tmp/test.php"#)?;
    nixos_shell.run_cmd_and_expect("php /tmp/test.php", "Hello from PHP")?;
    Ok(())
}

#[nixos_test]
fn php_inline(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(r#"php -r "echo 'Hello World';""#, "Hello World")?;
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
fn gcc_object_file(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'int add(int a, int b) { return a + b; }' > /tmp/add.c")?;
    nixos_shell.run_cmd("gcc -c /tmp/add.c -o /tmp/add.o")?;
    nixos_shell.run_cmd_and_expect("ls -l /tmp/add.o", "add.o")?;
    Ok(())
}

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
fn clang_llvm_ir(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'int main() { return 0; }' > /tmp/simple.c")?;
    nixos_shell.run_cmd("clang -S -emit-llvm /tmp/simple.c -o /tmp/simple.ll")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/simple.ll", "llvm.ident")?;
    Ok(())
}

// ============================================================================
// Version Control - Git
// ============================================================================

#[nixos_test]
fn git_init_status(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/git_test && cd /tmp/git_test && git init")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/git_test && git status", "On branch master")?;
    Ok(())
}

#[nixos_test]
fn git_commit(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/git_test2 && cd /tmp/git_test2 && git init")?;
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
fn git_branch(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/git_test3 && cd /tmp/git_test3 && git init")?;
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
// Build Tools - GNU Make
// ============================================================================

#[nixos_test]
fn make_simple(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/make_test")?;
    nixos_shell.run_cmd(r#"echo -e 'all:\n\techo "Hello from Make"' > /tmp/make_test/Makefile"#)?;
    nixos_shell.run_cmd_and_expect("cd /tmp/make_test && make", "Hello from Make")?;
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
    nixos_shell.run_cmd("cd /tmp/cmake_test && mkdir build && cd build && cmake ..")?;
    nixos_shell.run_cmd_and_expect(
        "cd /tmp/cmake_test/build && cmake --build .",
        "Built target hello",
    )?;
    nixos_shell.run_cmd_and_expect("/tmp/cmake_test/build/hello", "Hello from CMake")?;
    Ok(())
}

// ============================================================================
// Build Tools - Meson
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
