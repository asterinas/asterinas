{ pkgs, ... }:
let
  # On NixOS 26.05, python312.doc cannot be built. Remove this passthru so
  # the system-wide documentation output selection does not try to build it.
  py3 = pkgs.python312.overrideAttrs
    (old: { passthru = builtins.removeAttrs old.passthru [ "doc" ]; });
in {
  environment.systemPackages = [ py3 ];
  # Make the exact matching source tree available without a download.
  system.activationScripts.testFixtures = ''
    ln -sfT ${py3.src} /tmp/python3-src.tar.xz
  '';
}
