// SPDX-License-Identifier: MPL-2.0

//! The test suite for web servers and proxies applications on Asterinas NixOS.
//!
//! # Document maintenance
//!
//! An application's test suite and its "Verified Usage" section in Asterinas Book
//! should always be kept in sync.
//! So whenever you modify the test suite,
//! review the documentation and see if should be updated accordingly.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Web Servers
// ============================================================================

#[nixos_test]
fn nginx_server(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("cp -rL $(nginx -h 2>&1 | grep -o '/nix/store[^)]*') /tmp/nginx")?;
    nixos_shell.run_cmd(
        r"sed -i 's/^\(\s*listen\s*\)80;/\110.0.2.15:8000;/' /tmp/nginx/conf/nginx.conf",
    )?;
    nixos_shell.run_cmd("mkdir -p /var/log/nginx")?;

    nixos_shell.run_cmd("nginx -c /tmp/nginx/conf/nginx.conf")?;
    nixos_shell.run_cmd_and_expect("curl http://10.0.2.15:8000", "Welcome to nginx!")?;
    nixos_shell.run_cmd("nginx -s stop")?;
    Ok(())
}

#[nixos_test]
fn httpd_server(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell
        .run_cmd("cp -rL $(httpd -V 2>&1 | grep 'HTTPD_ROOT' | cut -d'\"' -f2) /tmp/httpd")?;
    nixos_shell
        .run_cmd(r"sed -i 's/^Listen 80$/Listen 10.0.2.15:8001/' /tmp/httpd/conf/httpd.conf")?;
    nixos_shell.run_cmd("sed -i 's/^User daemon$/User apache/' /tmp/httpd/conf/httpd.conf")?;
    nixos_shell.run_cmd("sed -i 's/^Group daemon$/Group apache/' /tmp/httpd/conf/httpd.conf")?;
    nixos_shell.run_cmd("groupadd -r apache 2>/dev/null")?;
    nixos_shell.run_cmd("useradd -r -g apache -s /sbin/nologin apache 2>/dev/null")?;

    nixos_shell.run_cmd("httpd -f /tmp/httpd/conf/httpd.conf")?;
    nixos_shell.run_cmd_and_expect("curl http://10.0.2.15:8001", "It works!")?;
    nixos_shell.run_cmd("httpd -f /tmp/httpd/conf/httpd.conf -k stop")?;
    Ok(())
}

#[nixos_test]
fn caddy_server(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/caddy")?;
    nixos_shell.run_cmd("echo 'Hello from Caddy' > /tmp/caddy/index.html")?;
    nixos_shell.run_cmd("cd /tmp/caddy")?;
    nixos_shell
        .run_cmd("caddy file-server --listen 10.0.2.15:8002 --browse > /tmp/caddy.log 2>&1 &")?;
    nixos_shell.run_cmd("sleep 3")?;
    nixos_shell.run_cmd_and_expect("curl http://10.0.2.15:8002", "Hello from Caddy")?;
    nixos_shell.run_cmd("pkill caddy")?;
    Ok(())
}
