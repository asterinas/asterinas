{ pkgs, ... }:
let py3 = pkgs.python312;
in {
  environment.systemPackages = [ py3 ];
  # Make the exact matching source tree available without a download.
  system.activationScripts.testFixtures = ''
    ln -sfT ${py3.src} /tmp/python3-src.tar.xz
  '';
}
