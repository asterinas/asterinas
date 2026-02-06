# Network Tools

## LDNS

[LDNS](https://www.nlnetlabs.nl/projects/ldns/) is a library for DNS programming with C.

### Installation

```nix
environment.systemPackages = pkgs.ldns;
```

### Verified Usage

#### DNS queries

```bash
# Basic DNS lookup
drill google.com

# Query specific record type
drill google.com A          # IPv4 address
drill google.com AAAA       # IPv6 address
drill google.com MX         # Mail exchange
drill google.com NS         # Name servers
drill google.com TXT        # Text records
drill google.com CNAME      # Canonical name

# Reverse DNS lookup
drill -x 8.8.8.8
```

## Whois

[Whois](https://packages.qa.debian.org/w/whois.html) queries domain registration information.

### Installation

```nix
environment.systemPackages = pkgs.whois;
```

### Verified Usage

#### Domain information

```bash
# Basic whois lookup
whois google.com

# Query specific whois server
whois -h whois.verisign-grs.com google.com

# Query IP address
whois 8.8.8.8
```

## Netcat

[Netcat](https://www.libressl.org/) is a networking utility for reading from and writing to network connections.

### Installation

```nix
environment.systemPackages = pkgs.netcat;
```

### Verified Usage

#### Network connections

```bash
# Basic TCP connection
nc hostname port

# Listen on specific port
nc -l 10.0.2.15 8080

# Send file over network
nc hostname port < file.txt

# Receive file over network
nc -l port > received_file.txt

# Zero-I/O mode (scanning)
nc -z hostname port
```

## Curl

[Curl](https://curl.se/) transfers data with URLs.

### Installation

```nix
environment.systemPackages = pkgs.curl;
```

### Verified Usage

#### HTTP client

```bash
# Basic GET request
curl https://api.github.com

# Download with specific filename
curl -o newname.txt https://example.com/file.txt
```

## Wget

[Wget](https://www.gnu.org/software/wget/) downloads files from the web.

### Installation

```nix
environment.systemPackages = pkgs.wget;
```

### Verified Usage

#### File download

```bash
# Download single file
wget https://example.com/file.zip

# Download with specific filename
wget -O newname.txt https://example.com/file.txt
```

## LFTP

[LFTP](https://lftp.yar.ru/) is a sophisticated file transfer program.

### Installation

```nix
environment.systemPackages = pkgs.lftp;
```

### Verified Usage

#### FTP client

```bash
# Connect to FTP server
lftp ftp://ftp.sjtu.edu.cn/ubuntu-cd/

# Connect to HTTP server
lftp http://ftp.sjtu.edu.cn/ubuntu-cd/

# Download single file
lftp -c "open ftp.sjtu.edu.cn; cd /ubuntu-cd; get robots.txt"
```

## Socat

[Socat](http://www.dest-unreach.org/socat/) is a multipurpose relay for bidirectional data transfer.

### Installation

```nix
environment.systemPackages = pkgs.socat;
```

### Verified Usage

#### Network relay

```bash
# Basic TCP connection
socat TCP:hostname:port -

# Listen on TCP port
socat TCP-LISTEN:8080,bind=10.0.2.15,fork -

# Echo server
socat TCP-LISTEN:6379,bind=10.0.2.15,reuseaddr,fork EXEC:cat

# HTTP server (simple)
socat TCP-LISTEN:8080,bind=10.0.2.15,crlf,reuseaddr,fork SYSTEM:"echo 'HTTP/1.0 200 OK'; echo; echo 'Hello World'"
```
