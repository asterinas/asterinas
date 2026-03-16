# Networking

This category covers network utilities, DNS/DHCP servers, VPN tools, and firewalls.

## Network Utilities

### Curl

[Curl](https://curl.se/) transfers data with URLs.

#### Installation

```nix
environment.systemPackages = [ pkgs.curl ];
```

#### Verified Usage

```bash
# Basic GET request
curl https://api.github.com

# Download with specific filename
curl -o newname.txt https://example.com/file.txt
```

### Wget

[Wget](https://www.gnu.org/software/wget/) downloads files from the web.

#### Installation

```nix
environment.systemPackages = [ pkgs.wget ];
```

#### Verified Usage

```bash
# Download single file
wget https://example.com/file.zip

# Download with specific filename
wget -O newname.txt https://example.com/file.txt
```

### TODO: OpenSSH

[OpenSSH](https://www.openssh.com/) is a suite of secure networking utilities.

### Rsync

[Rsync](https://rsync.samba.org/) is a fast and versatile file synchronization tool.

#### Installation

```nix
environment.systemPackages = [ pkgs.rsync ];
```

#### Verified Usage

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

### TODO: nmap

[nmap](https://nmap.org/) is a network security scanner.

### TODO: iproute2

[iproute2](https://wiki.linuxfoundation.org/networking/iproute2) provides utilities for controlling TCP/IP networking and traffic control.


### Netcat

[Netcat](https://www.libressl.org/) is a networking utility for reading from and writing to network connections.

#### Installation

```nix
environment.systemPackages = [ pkgs.netcat ];
```

#### Verified Usage

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

### LFTP

[LFTP](https://lftp.yar.ru/) is a sophisticated file transfer program.

#### Installation

```nix
environment.systemPackages = [ pkgs.lftp ];
```

#### Verified Usage

```bash
# Connect to FTP server
lftp ftp://ftp.sjtu.edu.cn/ubuntu-cd/

# Connect to HTTP server
lftp http://ftp.sjtu.edu.cn/ubuntu-cd/

# Download single file
lftp -c "open ftp.sjtu.edu.cn; cd /ubuntu-cd; get robots.txt"
```

### socat

[socat](http://www.dest-unreach.org/socat/) is a multipurpose relay for bidirectional data transfer.

#### Installation

```nix
environment.systemPackages = [ pkgs.socat ];
```

#### Verified Usage

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

## DNS & DHCP

### TODO: BIND

[BIND](https://www.isc.org/bind/) is the most widely-used DNS software on the Internet.

### TODO: dnsmasq

[dnsmasq](https://thekelleys.org.uk/dnsmasq/doc.html) is a lightweight DNS forwarder and DHCP server.

### LDNS

[LDNS](https://www.nlnetlabs.nl/projects/ldns/) is a library for DNS programming with C.

#### Installation

```nix
environment.systemPackages = [ pkgs.ldns ];
```

#### Verified Usage

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

### Whois

[Whois](https://packages.qa.debian.org/w/whois.html) queries domain registration information.

#### Installation

```nix
environment.systemPackages = [ pkgs.whois ];
```

#### Verified Usage

```bash
# Basic whois lookup
whois google.com

# Query specific whois server
whois -h whois.verisign-grs.com google.com

# Query IP address
whois 8.8.8.8
```

## VPN & Tunneling

### TODO: WireGuard

[WireGuard](https://www.wireguard.com/) is a simple, fast, and modern VPN.

### TODO: OpenVPN

[OpenVPN](https://openvpn.net/) is a flexible VPN solution using SSL/TLS.

## Firewalls

### TODO: iptables/nftables

[iptables](https://www.netfilter.org/projects/iptables/) and [nftables](https://www.netfilter.org/projects/nftables/) are userspace utilities for configuring packet filtering rules.

### TODO: UFW

[Uncomplicated Firewall (UFW)](https://launchpad.net/ufw) is a frontend for iptables designed for ease of use.
