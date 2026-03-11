# System Core

This category covers essential system components: shells, init systems, system monitoring tools, and core utilities.

## Shells

### Bash

[Bash](https://www.gnu.org/software/bash/) is the GNU Project's shell and command language, enabled by default in Asterinas NixOS.

### Zsh

[Zsh](https://www.zsh.org/) is a powerful shell with extensive customization options.

#### Installation

```nix
environment.systemPackages = [ pkgs.zsh ];
```

#### Verified Usage

```bash
# Execute a script
zsh script.sh
```

### Fish

[Fish](https://fishshell.com/) is a user-friendly shell with autosuggestions and web-based configuration.

#### Installation

```nix
environment.systemPackages = [ pkgs.fish ];
```

#### Verified Usage

```bash
# Execute a script
fish script.sh
```

## Init & Service Management

### Systemd

[Systemd](https://systemd.io/) is a software suite for system and service management, enabled by default in Asterinas NixOS.

## System Monitoring

### Htop

[Htop](https://htop.dev/) is an interactive process viewer.

#### Installation

```nix
environment.systemPackages = [ pkgs.htop ];
```

#### Verified Usage

```bash
# Start htop
htop

# Inside htop:
# Up/Down       - Select process
# Left/Right    - Move between columns or UI sections
# h or F1       - Help
# q             - Quit
```

### TODO: btop

[btop](https://github.com/aristocratos/btop) is resource monitor that shows usage and stats for processor, memory, disks, network and processes.

### fastfetch

[fastfetch](https://github.com/fastfetch-cli/fastfetch) is a system information tool similar to `neofetch`.

#### Installation

```nix
environment.systemPackages = [ pkgs.fastfetch ];
```

#### Verified Usage

```bash
# Show system information with default configuration
fastfetch

# Save current output to a file
fastfetch > fastfetch-output.txt
```

## Coreutils & Essential Tooling

### Coreutils

[Coreutils](https://www.gnu.org/software/coreutils/) includes basic file, shell and text manipulation utilities.

### Installation

```nix
environment.systemPackages = [ pkgs.coreutils ];
```

#### Verified Usage

```bash
# BLAKE2 checksum
b2sum file.txt > checksums.b2
b2sum --check checksums.b2

# Base64 encode/decode
base64 file.txt > encoded_file.b64
base64 -d encoded_file.b64

# Strip directory and suffix from filenames
basename /path/to/file.txt

# Concatenate and display files
cat file1.txt file2.txt

# Change file permissions
chmod 755 script.sh
chmod +x script.sh
chmod -R 755 directory/

# Run command or interactive shell with special root directory
chroot /newroot /bin/bash

# Copy files and directories
cp file1.txt file2.txt
cp -r dir1 dir2

# Split a file into sections determined by context lines
csplit file.txt '/pattern/' '{*}'

# Remove sections from each line of files
cut -d':' -f1 /etc/passwd

# Convert and copy a file
dd if=input.txt of=output.txt
dd if=/dev/zero of=disk.img bs=1M count=100

# Strip last component from file name
dirname /path/to/file.txt

# Display a line of text
echo "Hello World"

# Evaluate expressions
expr 2 + 3
expr 10 \* 5    # Escape * for shell

# Output the first part of files
head -n 20 file.txt

# Make links between files
ln -s target_file symlink
ln target_file hardlink

# Make directories
mkdir -p path/to/nested/dir

# Move (rename) files
mv old.txt new.txt
mv file.txt /destination/

# Dump files in octal and other formats
od file.txt
od -c file.txt  # Character format
od -x file.txt  # Hexadecimal format

# Merge lines of files
paste -d ',' file1.txt file2.txt    # Use comma as delimiter

# Print value of a symbolic link or canonical file name
readlink symlink

# Print the resolved path
realpath file.txt

# Remove files or directories
rm file.txt
rm -rf directory/

# Print a sequence of numbers
seq 0 2 10  # Start, increment, end

# SHA2 checksums
sha256sum file.txt > checksums.sha256
sha256sum --check checksums.sha256

# Display file or file system status
stat -c "%U %G" file.txt  # Custom format

# Flush file system buffers
sync

# Output the last part of files
tail -n 20 file.txt

# Change file timestamps
touch -t 202601011200 file.txt

# Print newline, word, and byte counts for each file
wc -l file.txt  # Lines only
wc -w file.txt  # Words only
wc -c file.txt  # Bytes only
```

### Util-linux

[Util-linux](https://www.kernel.org/pub/linux/utils/util-linux/) provides a set of system utilities for any Linux system.

#### Installation

```nix
environment.systemPackages = [ pkgs.util-linux ];
```

#### Verified Usage

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

### Procps

[Procps](https://gitlab.com/procps-ng/procps) provides system utilities for process management and system information.

#### Installation

```nix
environment.systemPackages = [ pkgs.procps ];
```

#### Verified Usage

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

### Findutils

[Findutils](https://www.gnu.org/software/findutils/) provides the basic directory searching utilities.

#### Installation

```nix
environment.systemPackages = [ pkgs.findutils ];
```

#### Verified Usage

```bash
# Search for files in a directory hierarchy
find /path -name "*.txt"           # Find by name
find /path -iname "*.TXT"          # Case insensitive
find /path -type f                 # Find files only
find /path -type d                 # Find directories only
find /path -type l                 # Find symbolic links only

# Execute commands on found files
find /path -name "*.tmp" -delete
find /path -name "*.log" -exec rm {} \;
find /path -name "*.bak" -exec cp {} {}.bak \;

# Build and execute command lines from standard input
find /path -name "*.txt" | xargs rm
find /path -name "*.log" | xargs -I {} cp {} {}.bak
```

### Less

[Less](https://www.greenwoodsoftware.com/less/) is a terminal pager program for viewing text files.

#### Installation

```nix
environment.systemPackages = [ pkgs.less ];
```

#### Verified Usage

```bash
# Opposite of more (better file viewer)
less file.txt

# Navigation commands (while in less):
# Basic movement:
# j or Enter    - Move down one line
# k             - Move up one line
# f or Space    - Forward one window
# b             - Backward one window
# d             - Forward half window
# u             - Backward half window
```
