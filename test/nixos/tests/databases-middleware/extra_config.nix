{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ sqlite redis valkey etcd influxdb ];
}
