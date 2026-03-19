{ config, lib, pkgs, ... }:
let
  jdk = pkgs.fetchFromGitHub {
    owner = "openjdk";
    repo = "jdk";
    rev = "jdk-21+7";
    hash = "sha256-PSva/2Q45hQIVQ3bB1p1TQ2d9S4/L7inXYxNacauLKE";
  };
in {
  programs.java.enable = true;
  environment.systemPackages = [ pkgs.jtreg ];
  environment.variables = { JTREG_HOME = "${pkgs.jtreg}"; };
  environment.loginShellInit = ''
    [ ! -e /tmp/jdk ] && ln -s ${jdk} /tmp/jdk
  '';
}
