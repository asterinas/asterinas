{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ skopeo ];
  virtualisation.podman.enable = true;
}
