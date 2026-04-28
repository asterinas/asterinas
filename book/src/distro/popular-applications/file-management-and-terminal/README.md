# File Management & Terminal

This category covers file managers, terminal emulators, archiving tools, and modern CLI utilities.

## Archiving & Compression

### Bzip2

[Bzip2](https://www.sourceware.org/bzip2) uses the Burrows-Wheeler algorithm for compression.

#### Installation

```nix
environment.systemPackages = [ pkgs.bzip2 ];
```

#### Verified Usage

```bash
# Compress file with bzip2
bzip2 file.txt          # Creates file.txt.bz2

# Decompress file with bunzip2
bunzip2 file.txt.bz2    # Restores file.txt

# Decompress to stdout
bzcat file.txt.bz2

# View compressed file with pager
bzless file.txt.bz2
bzmore file.txt.bz2

# Search in compressed file
bzgrep "pattern" file.txt.bz2
```

### Gzip

[Gzip](https://www.gnu.org/software/gzip/) is a popular data compression program.

#### Installation

```nix
environment.systemPackages = [ pkgs.gzip ];
```

#### Verified Usage

```bash
# Compress file with gzip
gzip file.txt       # Creates file.txt.gz

# Decompress file with gunzip
gunzip file.txt.gz  # Restores file.txt

# Decompress to stdout
zcat file.txt.gz

# View compressed file with pager
zless file.txt.gz
zmore file.txt.gz

# Search in compressed file
zgrep "pattern" file.txt.gz
```

### P7zip

[P7zip](https://p7zip.sourceforge.io/) is a port of 7-Zip to Unix-like systems.

#### Installation

```nix
environment.systemPackages = [ pkgs.p7zip ];
```

#### Verified Usage

```bash
# Create 7z archive
7z a archive.7z file1 file2 directory/

# Extract 7z archive
7z x archive.7z -o output_directory

# List contents of archive
7z l archive.7z

# Add files to existing archive
7z u archive.7z newfile.txt

# Test archive integrity
7z t archive.7z
```

### Tar

[Tar](https://www.gnu.org/software/tar/) creates and extracts archive files.

#### Installation

```nix
environment.systemPackages = [ pkgs.gnutar ];
```

#### Verified Usage

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

### Xz

[Xz](https://tukaani.org/xz/) provides high compression ratio using LZMA2 algorithm.

#### Installation

```nix
environment.systemPackages = [ pkgs.xz ];
```

#### Verified Usage

```bash
# Compress file with xz
xz file.txt         # Creates file.txt.xz

# Decompress file with unxz
unxz file.txt.xz    # Restores file.txt

# Decompress to stdout
xzcat file.txt.xz

# View compressed file with pager
xzless file.txt.xz
xzmore file.txt.xz

# Search in compressed file
xzgrep "pattern" file.txt.xz
```

### Zip

[Zip](https://www.info-zip.org/) is a file compression and archive utility.

#### Installation

```nix
environment.systemPackages = with pkgs; [ zip unzip ];
```

#### Verified Usage

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

## Terminal Multiplexers

### Screen

[Screen](https://www.gnu.org/software/screen/) is a terminal multiplexer.

#### Installation

```nix
environment.systemPackages = [ pkgs.screen ];
```

#### Verified Usage

```bash
# Start a new named session
screen -S mysession

# Start a daemon session in detached mode
screen -dmS mysession

# List existing sessions
screen -ls

# Reattach to a detached session
screen -r mysession

# Kill a session
screen -S mysession -X quit
```

## File Inspection

### File

[File](https://darwinsys.com/file/) determines file type by examining content.

#### Installation

```nix
environment.systemPackages = [ pkgs.file ];
```

#### Verified Usage

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

## Text Processing

### Bat

[Bat](https://github.com/sharkdp/bat) is a `cat` clone with syntax highlighting and Git integration.

#### Installation

```nix
environment.systemPackages = [ pkgs.bat ];
```

#### Verified Usage

```bash
# Display file with syntax highlighting
bat file.txt

# Print only plain text (no color)
bat --plain file.txt

# Show line numbers explicitly
bat -n file.txt
```

### Gawk

[Gawk](https://www.gnu.org/software/gawk/) is the GNU implementation of Awk programming language.

#### Installation

```nix
environment.systemPackages = [ pkgs.gawk ];
```

#### Verified Usage

```bash
# Use custom field separator
awk -F: '{print NR ": " $1}' /etc/passwd

# Print lines matching pattern
awk '/pattern/ {print}' file.txt

# Sum numbers in first column
awk '{sum += $1} END {print "Sum:", sum}' numbers.txt
```

### Sd

[Sd](https://github.com/chmln/sd) is an intuitive find-and-replace CLI tool.

#### Installation

```nix
environment.systemPackages = [ pkgs.sd ];
```

#### Verified Usage

```bash
# Replace text in files
sd "old" "new" *.txt
```

### Sed

[Sed](https://www.gnu.org/software/sed/) is the GNU implementation of stream editor.

#### Installation

```nix
environment.systemPackages = [ pkgs.gnused ];
```

#### Verified Usage

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

## Search & Filtering

### Eza

[eza](https://github.com/eza-community/eza) is a modern replacement for `ls`.

#### Installation

```nix
environment.systemPackages = [ pkgs.eza ];
```

#### Verified Usage

```bash
# List files in current directory
eza

# Long format including hidden files
eza -la

# List files only
eza -f

# List directories only
eza -D
```

### Fd

[Fd](https://github.com/sharkdp/fd) is a simple, fast alternative to `find`.

#### Installation

```nix
environment.systemPackages = [ pkgs.fd ];
```

#### Verified Usage

```bash
# Find files in current directory
fd

# Find files containing a pattern
fd pattern

# Execute a command for each match
fd pattern -x rm
```

### Fzf

[Fzf](https://github.com/junegunn/fzf) is a command-line fuzzy finder.

#### Installation

```nix
environment.systemPackages = [ pkgs.fzf ];
```

#### Verified Usage

```bash
# Print matches for the initial query and exit
find . -type f | fzf -f "pattern"
```

### Ripgrep

[Ripgrep](https://github.com/BurntSushi/ripgrep) is a fast line-oriented search tool.

#### Installation

```nix
environment.systemPackages = [ pkgs.ripgrep ];
```

#### Verified Usage

```bash
# Search for pattern in files
rg "pattern" /path/to/directory

# Case-insensitive search
rg -i "pattern"

# Search for a literal string (no regex)
rg -F "literal string"

# Show only matching filenames
rg -l "pattern"

# Show files that do NOT contain the pattern
rg -l "pattern" --files-without-match

# Show 1 lines of context before and after matches
rg -C 1 "pattern"
```

### The Silver Searcher

[The Silver Searcher](https://github.com/ggreer/the_silver_searcher) (`ag`) is a fast text search tool.

#### Installation

```nix
environment.systemPackages = [ pkgs.silver-searcher ];
```

#### Verified Usage

```bash
# Search in current directory
ag "pattern"

# Limit to specific file types
ag --cc "main"
```

### Tree

[Tree](http://mama.indstate.edu/users/ice/tree/) displays directory structures in a tree format.

#### Installation

```nix
environment.systemPackages = [ pkgs.tree ];
```

#### Verified Usage

```bash
# Show tree to depth 2
tree -L 2

# Show hidden files
tree -a
```

## Security & Backup

### Age

[Age](https://github.com/FiloSottile/age) is a simple, modern encryption tool.

#### Installation

```nix
environment.systemPackages = [ pkgs.age ];
```

#### Verified Usage

```bash
# Generate a key pair
age-keygen -o key.txt

# Encrypt and decrypt
age -r age1example... -o secrets.txt.age secrets.txt
age -d -i key.txt -o secrets.txt secrets.txt.age
```

### Crunch

[Crunch](https://sourceforge.net/projects/crunch-wordlist/) generates wordlists by pattern.

#### Installation

```nix
environment.systemPackages = [ pkgs.crunch ];
```

#### Verified Usage

```bash
# Generate passwords from length 4 to 6
crunch 4 6 abc123 -o wordlist.txt

# Generate a fixed-length pattern list
crunch 8 8 -t @@@@%%%% -o pattern.txt
```

### GnuPG

[GnuPG](https://gnupg.org/) provides OpenPGP encryption and signing.

#### Installation

```nix
environment.systemPackages = [ pkgs.gnupg ];
```

#### Verified Usage

```bash
# Quick generate without passphrase protection
gpg --batch --passphrase-fd 0 --quick-generate-key "Test User <test@example.com>" rsa2048 sign never <<< ""

# List public keys
gpg --list-keys

# Export public key
gpg --export --armor test@example.com > public_key.asc

# Export private key (be careful!)
gpg --export-secret-keys --armor test@example.com > private_key.asc

# Sign with specific key
gpg --sign --local-user test@example.com file.txt

# Verify and extract the original file
gpg --verify file.txt.gpg
gpg --output file.txt file.txt.gpg

# Encrypt with passphrase from command line
gpg --symmetric --passphrase "mysecretpassword" --output encrypted.txt --batch file.txt

# Decrypt with passphrase from command line
gpg --decrypt --passphrase "mysecretpassword" --batch encrypted.txt > decrypted.txt
```

### John the Ripper

[John the Ripper](https://www.openwall.com/john/) is a password security auditing tool.

#### Installation

```nix
environment.systemPackages = [ pkgs.john ];
```

#### Verified Usage

```bash
# Create test dictionary file
echo "password" > wordlist.txt
echo "123456" >> wordlist.txt
echo "admin" >> wordlist.txt
echo "welcome" >> wordlist.txt
echo "qwerty" >> wordlist.txt
echo "abc123" >> wordlist.txt
echo "password123" >> wordlist.txt

# Create a MD5 hash for testing
echo "482c811da5d5b4bc6d497ffa98491e38" > md5_hash.txt # Genuine MD5 hash of "password123"

# Basic password cracking with wordlist
john --wordlist=wordlist.txt --format=raw-md5 md5_hash.txt
```

### Restic

[Restic](https://restic.net/) is a fast, secure backup program.

#### Installation

```nix
environment.systemPackages = [ pkgs.restic ];
```

#### Verified Usage

```bash
# Initialize a repository
restic -r /tmp/restic-repo init

# Run a backup
restic -r /tmp/restic-repo backup ~/Documents

# List snapshots
restic -r /tmp/restic-repo snapshots
```

### Wipe

[Wipe](https://wipe.sourceforge.net/) securely erases files and directories.

#### Installation

```nix
environment.systemPackages = [ pkgs.wipe ];
```

#### Verified Usage

```bash
# Force wipe with zero-out pass (single pass of zeros)
wipe -f -z filename.txt

# Force wipe with specific number of passes
wipe -f -p 8 filename.txt
```
