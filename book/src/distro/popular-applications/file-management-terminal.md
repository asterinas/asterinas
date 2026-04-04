# File Management & Terminal Productivity

This category covers file managers, terminal emulators, archiving tools, and modern CLI utilities.

## File Managers

### TODO: Nautilus

[Nautilus](https://wiki.gnome.org/Apps/Files) is the default file manager for GNOME.

### TODO: Dolphin

[Dolphin](https://apps.kde.org/dolphin/) is the file manager for KDE Plasma.

### TODO: Thunar

[Thunar](https://docs.xfce.org/xfce/thunar/start) is the file manager for Xfce.

### TODO: Ranger

[Ranger](https://ranger.github.io/) is a terminal-based file manager with vi keybindings.

### TODO: Midnight Commander

[Midnight Commander (mc)](https://midnight-commander.org/) is a terminal-based visual file manager.

## Archiving & Compression

### GNU tar

[GNU tar](https://www.gnu.org/software/tar/) creates and extracts archive files.

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

### XZ Utils

[XZ Utils](https://tukaani.org/xz/) provides high compression ratio using LZMA2 algorithm.

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

### p7zip

[p7zip](https://p7zip.sourceforge.io/) is a port of 7-Zip to Unix-like systems.

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

## Terminal Emulators

### TODO: Alacritty

[Alacritty](https://alacritty.org/) is a fast, cross-platform GPU-accelerated terminal emulator.

### TODO: Kitty

[Kitty](https://sw.kovidgoyal.net/kitty/) is a fast, feature-rich terminal emulator with GPU support.

### TODO: WezTerm

[WezTerm](https://wezfurlong.org/wezterm/) is a GPU-accelerated terminal emulator and multiplexer.

### TODO: GNOME Terminal

[GNOME Terminal](https://help.gnome.org/users/gnome-terminal/stable/) is the default terminal emulator for GNOME.

## Terminal Multiplexers

### TODO: tmux

[tmux](https://github.com/tmux/tmux) is a terminal multiplexer that allows multiple terminal sessions in a single window.

### GNU Screen

[GNU Screen](https://www.gnu.org/software/screen/) is a terminal multiplexer.

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

## Modern CLI Utilities

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

### GNU sed

[GNU sed](https://www.gnu.org/software/sed/) is the GNU implementation of stream editor.

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

### fzf

[fzf](https://github.com/junegunn/fzf) is a command-line fuzzy finder.

#### Installation

```nix
environment.systemPackages = [ pkgs.fzf ];
```

#### Verified Usage

```bash
# Print matches for the initial query and exit
find . -type f | fzf -f "pattern"
```

### ripgrep

[ripgrep](https://github.com/BurntSushi/ripgrep) is a fast line-oriented search tool.

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

### fd

[fd](https://github.com/sharkdp/fd) is a simple, fast alternative to `find`.

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

# Find files with specific size
fd -S 1M

# Execute a command for each match
fd pattern -x rm
```

### bat

[bat](https://github.com/sharkdp/bat) is a `cat` clone with syntax highlighting and Git integration.

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

### eza

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
