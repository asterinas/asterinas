{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ sqlite etcd redis valkey influxdb ];
}
