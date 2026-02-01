{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ python312 ];
}
