{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ test-asterinas ];
}
