# Development Tools

## Git

[Git](https://git-scm.com/) is a distributed version control system.

### Installation

```nix
environment.systemPackages = pkgs.git;
```

### Verified Usage

#### Version control

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

## Vim

[Vim](https://www.vim.org/) is a highly configurable text editor.

### Installation

```nix
environment.systemPackages = pkgs.vim;
```

### Verified Usage

#### Text editing

```bash
# Open file in vim
vim filename.txt

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

## Nano

[Nano](https://www.nano-editor.org/) is a simple, user-friendly text editor.

### Installation

```nix
environment.systemPackages = pkgs.nano;
```

### Verified Usage

#### Text editing

```bash
# Open file in nano
nano filename.txt

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

## GCC

[GCC](https://gcc.gnu.org/) is the GNU Compiler Collection.

### Installation

```nix
environment.systemPackages = pkgs.gcc;
```

### Verified Usage

#### C compilation

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

## Gnumake

[Make](https://www.gnu.org/software/make/) automates build processes.

### Installation

```nix
environment.systemPackages = pkgs.gnumake;
```

### Verified Usage

#### Build automation

```bash
# Run make with specific target
make target_name

# Run make with specific Makefile
make -f Makefile.custom
```

## Perl

[Perl](https://www.perl.org/) is a highly capable, feature-rich programming language.

### Installation

```nix
environment.systemPackages = pkgs.perl;
```

### Verified Usage

#### Script execution

```bash
# Execute Perl script
perl script.pl

# Run Perl code directly
perl -e 'print "Hello World"'
```

## Lua

[Lua](https://www.lua.org/) is a powerful, efficient, lightweight, embeddable scripting language.

### Installation

```nix
environment.systemPackages = pkgs.lua;
```

### Verified Usage

#### Script execution

```bash
# Execute Lua script
lua script.lua

# Run Lua code directly
lua -e "print('Hello World')"
```

## Ruby

[Ruby](https://www.ruby-lang.org/) is a dynamic, open source programming language.

### Installation

```nix
environment.systemPackages = pkgs.ruby;
```

### Verified Usage

#### Script execution

```bash
# Execute Ruby script
ruby script.rb

# Run Ruby code directly
ruby -e "puts 'Hello World'"
```

## Octave

[Octave](https://www.gnu.org/software/octave/) is a scientific computing environment.

### Installation

```nix
environment.systemPackages = pkgs.octave;
```

### Verified Usage

#### Scientific computing

```bash
# Execute Octave script
octave script.m

# Run Octave code directly
octave --eval "disp('Hello World')"

# Matrix operations
octave --eval "[1 2; 3 4] * [5; 6]"

# Statistical calculations
octave --eval "mean([1 2 3 4 5])"
```

## Python3

[Python](https://www.python.org/) is a high-level programming language.

### Installation

```nix
environment.systemPackages = pkgs.python3;
```

### Verified Usage

#### Script execution

```bash
# Execute Python script
python3 script.py

# Run Python code directly
python3 -c "print('Hello World')"
```

## Zulu

[Zulu](https://www.azul.com/products/zulu/) is a certified OpenJDK build.

### Installation

```nix
environment.systemPackages = pkgs.zulu;
```

### Verified Usage

#### Java development

```bash
# Compile Java source file
javac HelloWorld.java

# Run Java program
java HelloWorld
```

## Go

[Go](https://go.dev/) is a programming language by Google.

### Installation

```nix
environment.systemPackages = pkgs.go;
```

### Verified Usage

#### Go development

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

## Rustup

[Rustup](https://www.rustup.rs/) is a toolchain manager for Rust.

### Installation

```nix
environment.systemPackages = pkgs.rustup;
```

### Verified Usage

#### Rust development

```bash
# Manage toolchains
rustup toolchain list
rustup install stable
rustup default stable

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
