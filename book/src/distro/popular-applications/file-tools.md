# File Tools

## Coreutils

[Coreutils](https://www.gnu.org/software/coreutils/) includes basic file, shell and text manipulation utilities.

### Installation

```nix
environment.systemPackages = pkgs.coreutils;
```

### Verified Usage

#### Basic file operations

```bash
# BLAKE2 checksum
b2sum file.txt
b2sum --check checksums.b2

# Base64 encode/decode
base64 file.txt
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
expr 10 \* 5				# Escape * for shell

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
od -c file.txt                    # Character format
od -x file.txt                    # Hexadecimal format

# Merge lines of files
paste -d ',' file1.txt file2.txt  # Use comma as delimiter

# Print value of a symbolic link or canonical file name
readlink symlink

# Print the resolved path
realpath file.txt

# Remove files or directories
rm file.txt
rm -rf directory/

# Print a sequence of numbers
seq 0 2 10                      # Start, increment, end

# SHA2 checksums
sha256sum file.txt
sha512sum --check checksums.sha512

# Display file or file system status
stat -c "%A %U %G %s" file.txt  # Custom format

# Flush file system buffers
sync

# Output the last part of files
tail -n 20 file.txt

# Change file timestamps
touch -t 202401011200 file.txt

# Print newline, word, and byte counts for each file
wc -l file.txt                  # Lines only
wc -w file.txt                  # Words only
wc -c file.txt                  # Bytes only
```

## Findutils

[Findutils](https://www.gnu.org/software/findutils/) provides the basic directory searching utilities.

### Installation

```nix
environment.systemPackages = pkgs.findutils;
```

### Verified Usage

#### File searching

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
find /path -name "*.bak" -exec mv {} {}.old \;

# Build and execute command lines from standard input
find /path -name "*.txt" | xargs rm
find /path -name "*.log" | xargs -I {} mv {} {}.old
```

## Diffutils

[Diffutils](https://www.gnu.org/software/diffutils/) compares files and produces output showing differences.

### Installation

```nix
environment.systemPackages = pkgs.diffutils;
```

### Verified Usage

#### File comparison

```bash
# Compare files line by line
diff -u file1.txt file2.txt

# Compare three files line by line
diff3 file1.txt file2.txt file3.txt     # Three-way comparison
diff3 -m file1.txt file2.txt file3.txt  # Merge conflicts
diff3 -E file1.txt file2.txt file3.txt  # Overlapping changes
```

## File

[File](https://darwinsys.com/file/) determines file type by examining content.

### Installation

```nix
environment.systemPackages = pkgs.file;
```

### Verified Usage

#### File type detection

```bash
# Determine file type
file filename.txt              # Basic file type detection
file document.pdf              # Identify PDF files
file image.jpg                 # Identify image files

# Detailed information
file -i filename.txt           # Show MIME type
file -b filename.txt           # Brief output (no filename)
file -L symlink                # Follow symlinks
```

## Tree

[Tree](https://oldmanprogrammer.net/source.php?dir=projects/tree) lists contents of directories in a tree-like format.

### Installation

```nix
environment.systemPackages = pkgs.tree;
```

### Verified Usage

#### Directory tree display

```bash
# List contents of directories in a tree-like format
tree /path/to/directory        # Display specific directory tree

# Basic tree options
tree -a                        # Show all files (including hidden)
tree -d                        # Show directories only
tree -f                        # Show full path prefix for each file
tree -h                        # Show file sizes in human readable format
```

## Less

[Less](https://www.greenwoodsoftware.com/less/) is a terminal pager program for viewing text files.

### Installation

```nix
environment.systemPackages = pkgs.less;
```

### Verified Usage

#### File viewing

```bash
# Opposite of more (better file viewer)
less filename.txt

# Navigation commands (while in less):
# Basic movement:
# j or Enter    - Move down one line
# k             - Move up one line
# f or Space    - Forward one window
# b             - Backward one window
# d             - Forward half window
# u             - Backward half window
```

## Gawk

[Gawk](https://www.gnu.org/software/gawk/) is the GNU implementation of Awk programming language.

### Installation

```nix
environment.systemPackages = pkgs.gawk;
```

### Verified Usage

#### Text processing

```bash
# Use custom field separator
awk -F: '{print NR ": " $1}' /etc/passwd

# Print lines matching pattern
awk '/pattern/ {print}' file.txt

# Sum numbers in first column
awk '{sum += $1} END {print "Sum:", sum}' numbers.txt
```

## Gnused

[Gnused](https://www.gnu.org/software/sed/) is the GNU implementation of stream editor.

### Installation

```nix
environment.systemPackages = pkgs.gnused;
```

### Verified Usage

#### Stream editing

```bash
# Print specific line numbers
sed -n '1,10p' file.txt

# Replace all occurrences with case insensitive
sed 's/old/new/gi' file.txt

# Delete specific line numbers
sed '1,5d' file.txt

# Insert text before line
sed '2i\New line inserted' file.txt

# Append text after line
sed '2a\Appended line' file.txt

# Replace entire line
sed '3c\Completely replaced line' file.txt
```

## Gnugrep

[Gnugrep](https://www.gnu.org/software/grep/) searches text using patterns.

### Installation

```nix
environment.systemPackages = pkgs.gnugrep;
```

### Verified Usage

#### Text searching

```bash
# Search for pattern in file
grep "pattern" file.txt

# Case insensitive search
grep -i "pattern" file.txt

# Recursive search in directory
grep -r "pattern" /path/to/directory

# Show line numbers
grep -n "pattern" file.txt
```

## Gnutar

[Gnutar](https://www.gnu.org/software/tar/) creates and extracts archive files.

### Installation

```nix
environment.systemPackages = pkgs.gnutar;
```

### Verified Usage

#### Archive operations

```bash
# Create tar archive
tar -cf archive.tar file1 file2 directory/

# Extract tar archive
tar -xf archive.tar

# List contents of archive
tar -tf archive.tar

# Append files to existing archive
tar -rf archive.tar newfile.txt

# Update files in archive
tar -uf archive.tar updated_file.txt
```

## Gzip

[Gzip](https://www.gnu.org/software/gzip/) is a popular data compression program.

### Installation

```nix
environment.systemPackages = pkgs.gzip;
```

### Verified Usage

#### Compression

```bash
# Compress file with gzip
gzip file.txt                  # Creates file.txt.gz

# Decompress file with gunzip
gunzip file.txt.gz             # Restores file.txt

# Decompress to stdout
zcat file.txt.gz

# View compressed file with pager
zless file.txt.gz
zmore file.txt.gz

# Search in compressed file
zgrep "pattern" file.txt.gz
```

## Bzip2

[Bzip2](https://www.sourceware.org/bzip2) uses the Burrows-Wheeler algorithm for compression.

### Installation

```nix
environment.systemPackages = pkgs.bzip2;
```

### Verified Usage

#### Compression

```bash
# Compress file with bzip2
bzip2 file.txt                 # Creates file.txt.bz2

# Decompress file with bunzip2
bunzip2 file.txt.bz2           # Restores file.txt

# Decompress to stdout
bzcat file.txt.bz2

# View compressed file with pager
bzless file.txt.bz2
bzmore file.txt.bz2

# Search in compressed file
bzgrep "pattern" file.txt.bz2
```

## Xz

[Xz](https://tukaani.org/xz/) provides high compression ratio using LZMA2 algorithm.

### Installation

```nix
environment.systemPackages = pkgs.xz;
```

### Verified Usage

#### Compression

```bash
# Compress file with xz
xz file.txt                    # Creates file.txt.xz

# Decompress file with unxz
unxz file.txt.xz               # Restores file.txt

# Decompress to stdout
xzcat file.txt.xz

# View compressed file with pager
xzless file.txt.xz
xzmore file.txt.xz

# Search in compressed file
xzgrep "pattern" file.txt.xz
```

## Zip

[Zip](https://www.info-zip.org/) is a file compression and archive utility.

### Installation

```nix
environment.systemPackages = with pkgs; [ zip unzip ];
```

### Verified Usage

#### Archive creation

```bash
# Create zip archive
zip archive.zip file1.txt file2.txt

# List contents of zip file
unzip -l archive.zip

# Extract all files
unzip archive.zip

# View file information in zip
zipinfo archive.zip

# Search for pattern in zip files
zipgrep "pattern" archive.zip
```

## Rsync

[Rsync](https://rsync.samba.org/) is a fast and versatile file synchronization tool.

### Installation

```nix
environment.systemPackages = pkgs.rsync;
```

### Verified Usage

#### File synchronization

```bash
# Sync local directories
rsync -av source/ destination/

# Delete files not in source
rsync -av --delete source/ destination/

# Show progress during transfer
rsync -av --progress source/ destination/

# Exclude specific files/patterns
rsync -av --exclude '*.tmp' source/ destination/

# Include only specific files
rsync -av --include '*.txt' --exclude '*' source/ destination/
```

## Wipe

[Wipe](https://wipe.sourceforge.net/) securely deletes files by overwriting them.

### Installation

```nix
environment.systemPackages = pkgs.wipe;
```

### Verified Usage

#### Secure deletion

```bash
# Wipe with zero-out pass (single pass of zeros)
wipe -z filename.txt

# Wipe with specific number of passes
wipe -p 8 filename.txt
```
