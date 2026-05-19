{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ go_1_24 ];
}
