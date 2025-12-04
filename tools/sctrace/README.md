# Syscall Compatibility Tracer

Syscall Compatibility Tracer (`sctrace`) is a powerful system call compatibility
verification tool that analyzes and validates system call against user-defined
patterns. Written in
[SCML (System Call Matching Language)](https://asterinas.github.io/book/kernel/linux-compatibility/syscall-flag-coverage/system-call-matching-language.html),
these patterns describe supported functionality of system calls.
`sctrace` supports both real-time monitoring of running programs and post-analysis of
existing trace logs, providing comprehensive insights into system call compatibility
with intuitive pattern matching and visual feedback.

## Features

- **Pattern-based filtering**: Define system call patterns using SCML syntax
- **Dual mode operation**:
  - Online mode: Real-time tracing of running programs
  - Offline mode: Analysis of existing strace log files
- **Multi-threaded support**: Automatic handling of multi-threaded program traces with syscall reconstruction.
When tracing multi-threaded programs, strace may split system calls across multiple lines due to thread interleaving.
`sctrace` automatically handles this reconstruction.
- **Multiple SCML files support**: Specify multiple `.scml` files as arguments to load all of them.
Each file maintains its own scope for bitflags and struct definitions, preventing cross-file pollution.

## How to build and install

### Prerequisites

- [**strace**](https://strace.io/) version 5.15 or higher (for online mode)
  - Debian/Ubuntu: `sudo apt install strace`
  - Fedora/RHEL: `sudo dnf install strace`
- Rust toolchain

### Build instructions

Make sure you have Rust installed, then build the project:

```bash
cargo build --release
```

The binary will be available at `target/release/sctrace`.

### Installation instructions

To install the binary (for example, to `/usr/local/bin`),
you can use:

```bash
sudo cp target/release/sctrace /usr/local/bin/
```

Or you can install from `crates.io` directly (Recommended):

```bash
cargo install sctrace
```

This will automatically download, build, and install the latest version of `sctrace`.

## Usage

### Basic Syntax

```bash
sctrace <SCML_FILE1> [SCML_FILE2 ...] [OPTIONS] -- [program] [args...]
```

### Options

- `--input <FILE>`: Specify input file for offline mode
- `--quiet`: Enable quiet mode (only show unsupported calls)

### Online Mode (Real-time tracing)

Trace a program in real-time:

```bash
sctrace pattern1.scml pattern2.scml -- ls -la
sctrace file_ops.scml network.scml --quiet -- ./my_program arg1 arg2
```

### Offline Mode (Log file analysis)

Analyze an existing strace log file:

```bash
sctrace pattern1.scml pattern2.scml --input trace.log
sctrace file_ops.scml network.scml --input trace.log --quiet
```

**Note**: When generating strace logs for offline analysis, use `-yy` and `-f` flags:

```bash
strace -yy -f -o trace.log ls -la
```

- `-yy`: Print paths associated with file descriptor arguments
- `-f`: Trace child processes created by fork/vfork/clone

## Examples

### Example 1: Basic File Operations

Create `file_ops.scml`:
```scml
openat(dirfd, flags = O_RDONLY | O_WRONLY | O_RDWR, mode);
read(fd, buf, count = <INTEGER>);
write(fd, buf, count = <INTEGER>);
close(fd);
```

Run:
```bash
sctrace file_ops.scml -- cat /etc/passwd
```

### Example 2: Network Operations

Create `network.scml`:
```scml
socket(domain = AF_INET | AF_INET6, type = SOCK_STREAM | SOCK_DGRAM, protocol);
connect(sockfd, addr, addrlen);
send(sockfd, buf, len, flags);
recv(sockfd, buf, len, flags);
```

Run:
```bash
sctrace network.scml -- curl http://example.com
```

### Example 3: Using Asterinas Compatibility Patterns

Use the provided directory [syscall-flag-coverage](../../book/src/kernel/linux-compatibility/syscall-flag-coverage) (work in progress) and
test with various commands:

```bash
# Monitor file system operations
sctrace $(find . -name "*.scml") -- tree .

# Monitor process information calls
sctrace $(find . -name "*.scml") -- top

# Monitor network operations
sctrace $(find . -name "*.scml") -- ping 127.0.0.1
```

### Example 4: Offline Analysis

```bash
# Generate trace log
strace -yy -f -o trace.log ls -la

# Analyze with sctrace
sctrace patterns.scml --input trace.log
```

## Output

`sctrace` provides colored output to distinguish between supported and unsupported system calls:

- **Supported calls**: Normal output (or hidden in quiet mode)
- **Unsupported calls**: Highlighted in red with "unsupported" message

### Example Output

```
openat(AT_FDCWD, "/etc/passwd", O_RDONLY) = 3
read(3, "root:x:0:0:root:/root:/bin/bash\n"..., 4096) = 1234
close(3) = 0
chmod("/tmp/test", 0755) (unsupported)
```

## Dependencies

- `clap`: Command-line argument parsing
- `regex`: Regular expression support
- `nom`: Parser combinator library
- `nix`: Unix system interface for process management

## Troubleshooting

### Permission Issues

For online tracing, you may need elevated privileges:

```bash
sudo sctrace patterns.scml -- target_program
```
