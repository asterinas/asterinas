# Security Tools

## GnuPG

[GnuPG](https://gnupg.org/) is a complete implementation of the OpenPGP standard for encrypting and signing data.

### Installation

```nix
environment.systemPackages = pkgs.gnupg;
```

### Verified Usage

#### Encryption and signing

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
gpg file.txt.gpg  # This will verify and output the original content

# Encrypt with passphrase from command line
gpg --symmetric --passphrase "mysecretpassword" --batch file.txt

# Decrypt with passphrase from command line
gpg --decrypt --passphrase "mysecretpassword" --batch file.txt.gpg > file.txt
```

## Crunch

[Crunch](https://sourceforge.net/projects/crunch-wordlist/) is a wordlist generator.

### Installation

```nix
environment.systemPackages = pkgs.crunch;
```

### Verified Usage

#### Wordlist generation

```bash
# Generate wordlist with specific length
crunch 4 4 abcdef

# Generate wordlist with specific pattern
crunch 6 6 -t a@@b%%

# Generate wordlist with start and end strings
crunch 4 4 abcdef -s abcd -e ffed
```

## John

[John the Ripper](https://github.com/openwall/john/) is a fast password cracker.

### Installation

```nix
environment.systemPackages = pkgs.john;
```

### Verified Usage

#### Password cracking

```bash
# Create test dictionary file
echo "password" > wordlist.txt
echo "123456" >> wordlist.txt
echo "admin" >> wordlist.txt
echo "password123" >> wordlist.txt

# Create a MD5 hash for testing
echo "482c811da5d5b4bc6d497ffa98491e38" > md5_hash.txt # Genuine MD5 hash of "password123"

# Basic password cracking with wordlist
john --wordlist=wordlist.txt --format=raw-md5 md5_hash.txt
```
