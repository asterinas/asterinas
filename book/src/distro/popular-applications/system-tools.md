# System Tools

## Bash

[Bash](https://www.gnu.org/software/bash/) is the GNU Project's shell and command language.

### Installation

Bash is included by default in NixOS. To ensure it's available, you can add it to your `configuration.nix`:

```nix
environment.systemPackages = pkgs.bash;
```

### Verified Usage

#### Basic commands

```bash
# Start interactive shell
bash

# Execute a script
bash script.sh
```

## Procps

[Procps](https://gitlab.com/procps-ng/procps) provides system utilities for process management and system information.

### Installation

```nix
environment.systemPackages = pkgs.procps;
```

### Verified Usage

#### Process management

```bash
# Display memory in human readable format
free -h

# Kill process by PID
kill 1234

# Display processes with custom format
ps -eo pid,ppid,cmd,pcpu,pmem

# Find processes with full command line
pgrep -f "python script.py"

# Kill processes by pattern
pkill firefox

# Display memory map of a process
pmap 1234

# Monitor processes in real-time
top

# Display system uptime
uptime
```

## Util-linux

[Util-linux](https://www.kernel.org/pub/linux/utils/util-linux/) provides a set of system utilities for any Linux system.

### Installation

```nix
environment.systemPackages = pkgs.util-linux;
```

### Verified Usage

#### System utilities

```bash
# Display system information
uname -a

# Display disk space usage
df -h

# Mount a file system
mount -t ext2 /dev/vdb /ext2

# Unmount a file system
umount /ext2

# Find mounted file systems
findmnt

# Display date in custom format
date +"%Y-%m-%d %H:%M:%S"

# Display calendar for specific month
cal 01 2026

# Display user and group information
id

# Display login history
last

# Display file in hexadecimal
hexdump -C file.bin

# Display where program is located
whereis ls
```

## Hostname

[Hostname](https://tracker.debian.org/pkg/hostname) displays or sets the host name or domain name.

### Installation

```nix
environment.systemPackages = pkgs.hostname;
```

### Verified Usage

#### Hostname management

```bash
# Display current hostname
hostname

# Set hostname temporarily
hostname new-hostname
```

## Which

[Which](https://www.gnu.org/software/which/) shows the full path of shell commands.

### Installation

```nix
environment.systemPackages = pkgs.which;
```

### Verified Usage

#### Command location

```bash
# Find all locations of a command
which -a python

# Explicitly search for normal binaries
which --skip-alias command
```

## Man-pages

[Man-pages](https://www.kernel.org/doc/man-pages/) provides manual pages for Linux kernel and C library interfaces.

### Installation

```nix
environment.systemPackages = pkgs.man-pages;
```

### Verified Usage

#### Manual page access

```bash
# Display manual page in specific section
man 1 ls        # User commands
man 2 read      # System calls
man 3 printf    # Library functions
man 4 tty       # Special files
man 5 fstab     # File formats
man 6 banner    # Games
man 7 regex     # Miscellaneous
man 8 mount     # System administration

# Display manual page without pager
man -P cat ls
```

## Texinfo

[Texinfo](https://www.gnu.org/software/texinfo/) is the official documentation format of the GNU project.

### Installation

```nix
environment.systemPackages = pkgs.texinfoInteractive;
```

### Verified Usage

#### Info documentation

```bash
# Display info documentation for a topic
info bash

# Navigate within info viewer:
# Space - Scroll forward
# Backspace - Scroll backward
# n - Next node
# p - Previous node
# u - Up node
# l - Last visited node
# g - Go to specific node
# s - Search forward
# q - Quit
```
