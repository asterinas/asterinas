{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    curl
    wget
    rsync
    netcat
    lftp
    socat
    ldns
    whois
  ];
}
