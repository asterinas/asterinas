{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    nginx
    apacheHttpd
    caddy
    haproxy
    traefik
    envoy
  ];
}
