{ pkgs, ... }:
let
  httpd_conf = pkgs.writeTextFile {
    name = "httpd.conf";
    text = ''
      ServerRoot "${pkgs.apacheHttpd}"
      Listen 10.0.2.15:8000
      PidFile "/tmp/httpd/httpd.pid"
      ErrorLog "/tmp/httpd/error.log"
      ServerName localhost

      LoadModule mpm_event_module modules/mod_mpm_event.so
      LoadModule authz_core_module modules/mod_authz_core.so
      LoadModule authz_host_module modules/mod_authz_host.so
      LoadModule dir_module modules/mod_dir.so
      LoadModule unixd_module modules/mod_unixd.so

      User apache
      Group apache

      DocumentRoot "/tmp/httpd/html"
      DirectoryIndex index.html

      <Directory "/tmp/httpd/html">
        AllowOverride None
        Require all granted
      </Directory>
    '';
  };

  nginx_conf = pkgs.writeTextFile {
    name = "nginx.conf";
    text = ''
      worker_processes 1;
      pid /tmp/nginx/nginx.pid;
      error_log /tmp/nginx/error.log;
      events {}
      http {
        access_log /tmp/nginx/access.log;
        server {
          listen 10.0.2.15:8001;
          location / {
            default_type text/plain;
            return 200 "Hello from NGINX";
          }
        }
      }
    '';
  };

  openresty_conf = pkgs.writeTextFile {
    name = "openresty.conf";
    text = ''
      worker_processes 1;
      pid /tmp/openresty/openresty.pid;
      error_log /tmp/openresty/error.log;
      events {}
      http {
        access_log /tmp/openresty/access.log;
        server {
              listen 10.0.2.15:8002;
              location / {
                  default_type text/html;
                  content_by_lua_block {
                      ngx.say("Hello from Openresty")
                  }
              }
          }
      }
    '';
  };
in
{
  environment.systemPackages = with pkgs; [
    apacheHttpd
    caddy
    nginx
    openresty
  ];

  users.groups.apache = { };

  users.users.apache = {
    group = "apache";
    isSystemUser = true;
    shell = "/sbin/nologin";
  };

  system.activationScripts.testFixtures = ''
    mkdir -p /tmp/httpd /tmp/nginx /tmp/openresty

    ln -sfT ${httpd_conf} /tmp/httpd/httpd.conf
    ln -sfT ${nginx_conf} /tmp/nginx/nginx.conf
    ln -sfT ${openresty_conf} /tmp/openresty/openresty.conf
  '';
}
