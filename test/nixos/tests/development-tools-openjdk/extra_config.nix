{ pkgs, ... }:
let
  jdk = pkgs.openjdk21;
  jtreg = pkgs.jtreg;
in
{
  environment.systemPackages = [ jtreg ];
  environment.variables = {
    JTREG_HOME = "${jtreg}";
  };

  programs.java.package = jdk;
  programs.java.enable = true;

  system.activationScripts.testFixtures = ''
    ln -sfT ${jdk.src} /tmp/jdk-src
  '';
}
