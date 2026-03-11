# Development Tools

This category covers programming language runtimes, version control, build tools, editors, and debugging utilities.

## Programming Language Runtimes

### Python3

[Python](https://www.python.org/) is a high-level programming language.

#### Installation

```nix
environment.systemPackages = [ pkgs.python3 ];
```

#### Verified Usage

```bash
# Execute Python script
python3 script.py

# Run Python code directly
python3 -c "print('Hello World')"
```

### Node.js

[Node.js](https://nodejs.org/) is a JavaScript runtime built on Chrome's V8 engine.

#### Installation

```nix
environment.systemPackages = [ pkgs.nodejs ];
```

#### Verified Usage

```bash
# Run Node.js script
node script.js

# Run Node.js code directly
node -e "console.log('Hello World')"
```

### Go

[Go](https://golang.org/) is an open source programming language designed for simplicity and reliability.

#### Installation

```nix
environment.systemPackages = [ pkgs.go ];
```

#### Verified Usage

```bash
# Initialize new Go module
go mod init module-name

# Build Go program
go build main.go

# Run Go program
go run main.go

# Format source
go fmt main.go

# Clean build artifacts
go clean
```

### Rustup

[Rustup](https://www.rustup.rs/) is a toolchain manager for Rust.

#### Installation

```nix
environment.systemPackages = [ pkgs.rustup ];
```

#### Verified Usage

```bash
# Manage toolchains
rustup toolchain list
rustup install stable
rustup default stable
```

### OpenJDK

[OpenJDK](https://openjdk.java.net/) is a free and open-source implementation of the Java Platform.

#### Installation

```nix
environment.systemPackages = [ pkgs.openjdk ];
```

#### Verified Usage

```bash
# Compile Java source file
javac HelloWorld.java

# Run Java program
java HelloWorld
```

### Ruby

[Ruby](https://www.ruby-lang.org/) is a dynamic, open source programming language.

#### Installation

```nix
environment.systemPackages = [ pkgs.ruby ];
```

#### Verified Usage

```bash
# Execute Ruby script
ruby script.rb

# Run Ruby code directly
ruby -e "puts 'Hello World'"
```

### Perl

[Perl](https://www.perl.org/) is a highly capable, feature-rich programming language.

#### Installation

```nix
environment.systemPackages = [ pkgs.perl ];
```

#### Verified Usage

```bash
# Execute Perl script
perl script.pl

# Run Perl code directly
perl -e 'print "Hello World"'
```

### Lua

[Lua](https://www.lua.org/) is a powerful, efficient, lightweight, embeddable scripting language.

#### Installation

```nix
environment.systemPackages = [ pkgs.lua ];
```

#### Verified Usage

```bash
# Execute Lua script
lua script.lua

# Run Lua code directly
lua -e "print('Hello World')"
```

### PHP

[PHP](https://www.php.net/) is a popular general-purpose scripting language for web development.

#### Installation

```nix
environment.systemPackages = [ pkgs.php ];
```

#### Verified Usage

```bash
# Execute PHP script
php script.php

# Run PHP code directly
php -r "echo 'Hello World';"
```

### GCC

[GCC](https://gcc.gnu.org/) is the GNU Compiler Collection.

#### Installation

```nix
environment.systemPackages = [ pkgs.gcc ];
```

#### Verified Usage

```bash
# Create object file only
gcc -c source.c

# Compile with output name
gcc -o program source.c

# Compile with debug information
gcc -g source.c -o program

# Link object files
gcc file1.o file2.o -o program
```

#### Clang

[Clang](https://clang.llvm.org/) is a C language family frontend for the LLVM compiler.

#### Installation

```nix
environment.systemPackages = [ pkgs.llvmPackages.clangUseLLVM ];
```

#### Verified Usage

```bash
# Generate LLVM IR instead of native code
clang -S -emit-llvm hello.c -o hello.ll

# Show included headers
clang -H -c hello.c -o /dev/null

# Compile with optimization
clang -O2 hello.c -o hello
```

## Version Control

### Git

[Git](https://git-scm.com/) is a distributed version control system.

#### Installation

```nix
environment.systemPackages = [ pkgs.git ];
```

#### Verified Usage

```bash
# Clone existing repository
git clone https://github.com/user/repo.git

# Check repository status
git status

# View commit history
git log

# Create and switch to new branch
git checkout -b new-feature

# View differences
git diff

# Add files to staging area
git add file.txt

# Commit changes
git commit -m "Commit message"

# Push changes to remote
git push origin main
```

## Build Tools

### GNU Make

[GNU Make](https://www.gnu.org/software/make/) automates build processes.

#### Installation

```nix
environment.systemPackages = [ pkgs.gnumake ];
```

#### Verified Usage

```bash
# Run make with specific target
make target_name

# Run make with specific Makefile
make -f Makefile.custom
```

### CMake

[CMake](https://cmake.org/) is a cross-platform build system generator.

#### Installation

```nix
environment.systemPackages = [ pkgs.cmake ];
```

#### Verified Usage

```bash
# Generate build files
cmake .

# Build project
cmake --build .

# Clean build
cmake --build . --target clean
```

### Meson

[Meson](https://mesonbuild.com/) is a fast and user-friendly build system.

#### Installation

```nix
environment.systemPackages = with pkgs; [
    (python3.withPackages (p: [ p.meson ]))
    meson
    ninja
];
```

#### Verified Usage

```bash
# Initialize project
meson setup builddir

# Build project
meson compile -C builddir

# Run output from build directory
./builddir/hello
```

### Cargo

[Cargo](https://doc.rust-lang.org/cargo/) is the Rust package manager and build system.

#### Installation

```nix
environment.systemPackages = [ pkgs.cargo ];
```

#### Verified Usage

```bash
# Create new Rust project
cargo new my_project

# Fast compilation check
cargo check

# Build project
cargo build

# Run project
cargo run

# Test project
cargo test
```

### TODO: npm

[npm](https://www.npmjs.com/) is the package manager for Node.js.

## Editors & IDEs

### Vim

[Vim](https://www.vim.org/) is a highly configurable text editor for efficient text editing.

#### Installation

```nix
environment.systemPackages = [ pkgs.vim ];
```

#### Verified Usage

```bash
# Open file in vim
vim file.txt

# Basic navigation (in normal mode):
# h,j,k,l       - Move left, down, up, right
# w,W           - Move to next word
# b,B           - Move to previous word
# 0             - Move to beginning of line
# $             - Move to end of line
# gg            - Go to first line
# G             - Go to last line
# :10           - Go to line 10

# Editing modes:
# i             - Insert mode before cursor
# a             - Insert mode after cursor
# o             - Open new line below
# O             - Open new line above
# Esc           - Return to normal mode

# Saving and quitting:
# :w            - Save file
# :q            - Quit
# :wq or :x     - Save and quit
# :q!           - Quit without saving
# ZZ            - Save and quit
```

### Neovim

[Neovim](https://neovim.io/) is a hyperextensible Vim-based text editor.

#### Installation

```nix
environment.systemPackages = [ pkgs.neovim ];
```

#### Verified Usage

```bash
# Open file in neovim
nvim file.txt
```

### Emacs

[GNU Emacs](https://www.gnu.org/software/emacs/) is a customizable, extensible text editor.

#### Installation

```nix
environment.systemPackages = [ pkgs.emacs ];
```

#### Verified Usage

```bash
# Open file in Emacs (terminal)
emacs file.txt

# Basic navigation (C = Ctrl, M = Alt/Esc):
# C-f           - Move forward (right)
# C-b           - Move backward (left)
# C-n           - Next line (down)
# C-p           - Previous line (up)
# C-a           - Beginning of line
# C-e           - End of line
# M-f           - Forward one word
# M-b           - Backward one word
# M-<           - Beginning of buffer
# M->           - End of buffer
# M-g g         - Go to line (prompt for line number)

# Editing:
# C-d           - Delete character under cursor
# DEL (Backspace) - Delete character before cursor
# M-d           - Delete word forward
# C-k           - Kill (cut) to end of line
# C-y           - Yank (paste)
# C-space       - Start selection (set mark)
# C-w           - Cut selected region
# M-w           - Copy selected region

# Saving and quitting:
# C-x C-s       - Save current buffer
# C-x C-w       - Save as (prompt for filename)
# C-x C-c       - Save modified buffers and exit Emacs
# C-g           - Cancel current command/prompt
```

### Nano

[Nano](https://www.nano-editor.org/) is a simple, user-friendly terminal-based text editor.

#### Installation

```nix
environment.systemPackages = [ pkgs.nano ];
```

#### Verified Usage

```bash
# Open file in nano
nano file.txt

# Basic navigation:
# Ctrl+K        - Cut line
# Ctrl+U        - Paste line
# Ctrl+6        - Mark text (start selection)
# Ctrl+W        - Search text
# Ctrl+\        - Search and replace
# Ctrl+G        - Show help
# Ctrl+C        - Show cursor position
# Ctrl+_        - Go to line

# File operations:
# Ctrl+O        - Write file (save)
# Ctrl+X        - Exit nano
# Ctrl+R        - Read file (insert file)
# Ctrl+T        - Run spell checker
# Ctrl+J        - Justify paragraph
```

### TODO: VS Code

[Visual Studio Code](https://code.visualstudio.com/) is a lightweight but powerful source code editor.

## Debugging & Profiling

### TODO: GDB

[GNU Debugger (GDB)](https://www.gnu.org/software/gdb/) is a debugger for multiple programming languages.

### TODO: strace

[strace](https://strace.io/) is a diagnostic tool for tracing system calls.

### TODO: perf

[Linux perf](https://perf.wiki.kernel.org/) is a performance analysis tool for Linux.

### TODO: Valgrind

[Valgrind](https://valgrind.org/) is an instrumentation framework for building dynamic analysis tools.
