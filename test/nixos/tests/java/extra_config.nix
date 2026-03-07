{ config, lib, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ jtreg git ];
  environment.variables = { JTREG_HOME = "${pkgs.jtreg}"; };
  programs.java.enable = true;
}
