// SPDX-License-Identifier: MPL-2.0

//! The test suite for web servers and proxies applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Web Servers
// ============================================================================

#[nixos_test]
fn httpd_serve_webpage(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/httpd/html")?;
    nixos_shell.run_cmd("echo 'Hello from httpd' > /tmp/httpd/html/index.html")?;

    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "httpd -f /tmp/httpd/httpd.conf",
            CommandCheck::new("curl http://10.0.2.15:8000", "Hello from httpd"),
            "httpd -f /tmp/httpd/httpd.conf -k stop",
            CommandCheck::new("test ! -f /tmp/httpd/httpd.pid && echo stopped", "stopped"),
        ),
        |shell| shell.run_cmd_and_expect("curl http://10.0.2.15:8000", "Hello from httpd"),
    )?;

    Ok(())
}

#[nixos_test]
fn nginx_serve_webpage(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "nginx -c /tmp/nginx/nginx.conf",
            CommandCheck::new("curl http://10.0.2.15:8001", "Hello from NGINX"),
            "nginx -c /tmp/nginx/nginx.conf -s stop",
            CommandCheck::new("test ! -f /tmp/nginx/nginx.pid && echo stopped", "stopped"),
        ),
        |shell| shell.run_cmd_and_expect("curl http://10.0.2.15:8001", "Hello from NGINX"),
    )?;

    Ok(())
}

#[nixos_test]
fn openresty_serve_webpage(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "openresty -c /tmp/openresty/openresty.conf",
            CommandCheck::new("curl http://10.0.2.15:8002", "Hello from Openresty"),
            "openresty -s stop",
            CommandCheck::new(
                "test ! -f /tmp/openresty/openresty.pid && echo stopped",
                "stopped",
            ),
        ),
        |shell| shell.run_cmd_and_expect("curl http://10.0.2.15:8002", "Hello from Openresty"),
    )?;

    Ok(())
}

#[nixos_test]
fn caddy_serve_webpage(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/caddy")?;
    nixos_shell.run_cmd("echo 'Hello from Caddy' > /tmp/caddy/index.html")?;

    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "caddy file-server --root /tmp/caddy --listen 10.0.2.15:8003 > /tmp/caddy.log 2>&1 &",
            CommandCheck::new("curl http://10.0.2.15:8003", "Hello from Caddy"),
            "pkill caddy",
            CommandCheck::new("! pgrep -x caddy >/dev/null && echo stopped", "stopped"),
        ),
        |shell| shell.run_cmd_and_expect("curl http://10.0.2.15:8003", "Hello from Caddy"),
    )?;

    Ok(())
}
