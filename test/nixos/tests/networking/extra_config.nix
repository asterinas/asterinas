{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [
    curl
    lftp
    netcat
    rclone
    rsync
    socat
    wget
    ldns
    whois
  ];
}
