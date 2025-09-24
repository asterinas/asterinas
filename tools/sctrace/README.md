# Syscall-Compliance-Trace

Syscall-Compliance-Trace (sctrace) is a powerful system call compliance
verification tool that analyzes and validates system call against user-defined
compliance patterns. Written in SCML (System Call Matching Language), these
patterns describe supported functionality of system calls.
sctrace supports both real-time monitoring of running programs and post-analysis of
existing trace logs, providing comprehensive insights into system call compliance
with intuitive pattern matching and visual feedback.

## Features

- **Pattern-based filtering**: Define system call patterns using SCML syntax
- **Dual mode operation**:
  - Online mode: Real-time tracing of running programs
  - Offline mode: Analysis of existing strace log files
- **Quiet mode**: Minimal output showing only unsupported calls
- **Multi-threaded support**: Automatic handling of multi-threaded program traces with syscall reconstruction

## Installation

Make sure you have Rust installed, then build the project:

```bash
cargo build --release
```

The binary will be available at `target/release/sctrace`.

## Prerequisites

- **strace** version 5.15 or higher (for online mode)
- Rust toolchain

## Usage

### Basic Syntax

```bash
sctrace <scml_file> [OPTIONS] -- [program] [args...]
```

### Online Mode (Real-time tracing)

Trace a program in real-time:

```bash
sctrace patterns.scml ls -la
sctrace patterns.scml --quiet -- ./my_program arg1 arg2
```

### Offline Mode (Log file analysis)

Analyze an existing strace log file:

```bash
sctrace patterns.scml --input trace.log
sctrace patterns.scml --input trace.log --quiet
```

### Options

- `--input <FILE>`: Specify input file for offline mode
- `--quiet`: Enable quiet mode (only show unsupported calls)

## SCML (System Call Matching Language)

SCML is a domain-specific language for defining system call patterns. For more
detailed information about SCML syntax and features, please refer to the
[official SCML documentation](https://asterinas.github.io/book/kernel/linux-compatibility/limitations-on-system-calls/system-call-matching-language.html).

It supports:

### Basic Syntax

```scml
// Basic system call pattern
read(fd, buf, count);

// Constrained parameters
open(pathname, flags = O_RDONLY | O_WRONLY, mode);

// Comments (C-style)
// This is a comment
```

### Built-in Types

- `<INTEGER>`: Constrains parameter to integer values
- `<PATH>`: Constrains parameter to file path patterns

### Named Bitflags

Define reusable flag sets:

```scml
access_mode = O_RDONLY | O_WRONLY | O_RDWR;
open_flags = O_CREAT | O_EXCL | O_TRUNC;

open(pathname = <PATH>, flags = <access_mode> | <open_flags>, mode);
```

### Struct Patterns

Define structured data patterns:

```scml
struct stat = {
    st_mode = <INTEGER>,
    st_size = <INTEGER>,
    ..
};

stat(pathname = <PATH>, statbuf = <stat>);
```

### Array Patterns

Define array constraints:

```scml
poll(fds = [{ fd, events = POLLIN | POLLOUT, .. }], nfds, timeout);
```

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

Use the provided [asterinas.scml](../../book/src/kernel/linux-compatibility/limitations-on-system-calls/asterinas.scml) (work in progress) and
test with various commands:

```bash
# Monitor file system operations
sctrace asterinas.scml -- tree .

# Monitor process information calls
sctrace asterinas.scml -- top

# Monitor network operations
sctrace asterinas.scml -- ping 127.0.0.1
```

### Example 4: Offline Analysis

```bash
# Generate trace log
strace -o trace.log ls -la

# Analyze with sctrace
sctrace patterns.scml --input trace.log
```

## strace Output Format Support

sctrace supports parsing various strace output formats, including multi-threaded program traces.

### Single-threaded Output

Standard strace output for single-threaded programs:
```
openat(AT_FDCWD, "/etc/passwd", O_RDONLY) = 3
read(3, "root:x:0:0:root:/root:/bin/bash\n"..., 4096) = 1234
close(3) = 0
```

### Multi-threaded Output

When tracing multi-threaded programs, strace may split system calls across multiple lines due to thread interleaving. sctrace automatically handles this reconstruction:

**Blocked syscall format:**
```
1234 openat(AT_FDCWD, "/path/to/file", O_RDONLY <unfinished ...>
```

**Resumed syscall format:**
```
1234 <... openat resumed>) = 3
```

**Automatic reconstruction:**
sctrace internally reconstructs these into complete syscalls:
```
1234 openat(AT_FDCWD, "/path/to/file", O_RDONLY) = 3
```

### Signal and Exit Lines

sctrace automatically skips signal and process exit information:

**Signal lines (skipped):**
```
--- SIGCHLD {si_signo=SIGCHLD, si_code=CLD_EXITED, si_pid=1234, ...} ---
```

**Exit status lines (skipped):**
```
+++ exited with 0 +++
```

## Output

sctrace provides colored output to distinguish between supported and unsupported system calls:

- **Supported calls**: Normal output (or hidden in quiet mode)
- **Unsupported calls**: Highlighted in red with "unsupported" message

### Example Output

```
openat(AT_FDCWD, "/etc/passwd", O_RDONLY) = 3
read(3, "root:x:0:0:root:/root:/bin/bash\n"..., 4096) = 1234
close(3) = 0
chmod("/tmp/test", 0755) (unsupported)
```

## Project Structure

```
src/
├── lib.rs              # Library exports
├── main.rs             # Main application entry point
├── parameter.rs        # Command-line argument parsing
├── scml_parser.rs      # SCML language parser
├── scml_matcher.rs     # Pattern matching engine
├── strace_parser.rs    # strace output parser
└── trace.rs            # Trace iteration (online/offline)
```

## Dependencies

- `clap`: Command-line argument parsing
- `regex`: Regular expression support
- `nom`: Parser combinator library
- `tempfile`: Temporary file handling (dev dependency)

## Testing

Run the test suite:

```bash
cargo test
```

Run with verbose output:

```bash
cargo test -- --nocapture
```

## Troubleshooting

### strace Version Issues

If you encounter version-related errors, ensure strace 5.15+ is installed:

```bash
strace --version
```

### Permission Issues

For online tracing, you may need elevated privileges:

```bash
sudo sctrace patterns.scml -- target_program
```