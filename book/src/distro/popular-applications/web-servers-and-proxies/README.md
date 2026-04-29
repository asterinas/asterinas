# Web Servers & Proxies

This category covers web servers and reverse proxies/load balancers.

## Web Servers

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

### NGINX

[NGINX](https://nginx.org/) is a high-performance web server, reverse proxy, and load balancer.

#### Installation

```nix
environment.systemPackages = [ pkgs.nginx ];
```

#### Verified Usage

```bash
# Start server with a configuration file
nginx -c /tmp/nginx.conf
```

### OpenResty

[OpenResty](https://openresty.org/) is an Nginx-based web platform with LuaJIT support.

#### Installation

```nix
environment.systemPackages = [ pkgs.openresty ];
```

#### Verified Usage

```bash
# Start OpenResty with a custom prefix and config
openresty -p /tmp/openresty -c /tmp/openresty.conf
```
