{ lib, pkgs, ... }:

let
  upstream = "${pkgs.apparmor-parser.src}/tests/regression/apparmor";
in
{
  environment.systemPackages = [
    pkgs.apparmor-parser
    pkgs.perl
    pkgs.util-linux
    pkgs.zstd
  ];

  environment.etc."apparmor/parser.conf".source = "${pkgs.apparmor-parser.src}/parser/parser.conf";

  environment.etc."apparmor-tests".source = pkgs.runCommand "apparmor-e2e-tests-5.0.2" { } ''
    mkdir -p "$out"
    cp ${pkgs.apparmor-parser.src}/LICENSE "$out/LICENSE"
    for script in e2e.sh prologue.inc epilogue.inc mkprofile.pl; do
      cp "${upstream}/$script" "$out/"
    done
  '';

  # Keep the upstream scripts unchanged, including their interpreter/tool paths.
  system.activationScripts.apparmorTestTools = ''
    mkdir -p /bin /usr/bin /var/cache/apparmor
    ln -sfn ${pkgs.perl}/bin/perl /usr/bin/perl
    ln -sfn ${pkgs.coreutils}/bin/pwd /bin/pwd
    ln -sfn ${pkgs.coreutils}/bin/true /bin/true
  '';

  system.systemBuilderCommands = lib.mkAfter ''
    sed -i 's_ -- root=_ lsm=apparmor,yama -- root=_' $out/kernel-params
  '';
}
