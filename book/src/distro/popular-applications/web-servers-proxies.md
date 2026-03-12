# Web Servers & Proxies

This category covers web servers and reverse proxies/load balancers.

## Web Servers

### Nginx

[Nginx](https://nginx.org/) is a high-performance web server, reverse proxy, and load balancer.

#### Installation

```nix
environment.systemPackages = [ pkgs.nginx ];
```

#### Verified Usage

```bash
# Start server with a configuration file
nginx -c /tmp/nginx.conf
```

### Apache HTTP Server

[Apache HTTP Server](https://httpd.apache.org/) is a widely-used web server software.

#### Installation

```nix
environment.systemPackages = [ pkgs.apacheHttpd ];
```

#### Verified Usage

```bash
# Start server with a configuration file
httpd -f /tmp/httpd.conf
```

### Caddy

[Caddy](https://caddyserver.com/) is a modern web server with automatic HTTPS.

#### Installation

```nix
environment.systemPackages = [ pkgs.caddy ];
```

#### Verified Usage

```bash
# Start a file server
caddy file-server --listen 10.0.2.15:8002
```

## Reverse Proxies & Load Balancers

### TODO: HAProxy

[HAProxy](http://www.haproxy.org/) is a reliable, high-performance TCP/HTTP load balancer.

### TODO: Traefik

[Traefik](https://traefik.io/) is a modern HTTP reverse proxy and load balancer with automatic service discovery.

### TODO: Envoy

[Envoy](https://www.envoyproxy.io/) is a high-performance proxy designed for service mesh architectures.
