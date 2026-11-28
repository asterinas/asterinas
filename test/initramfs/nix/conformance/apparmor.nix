{
  lib,
  stdenvNoCC,
  pkgs,
  conformanceSrc,
}:

let
  runtimePath = lib.makeBinPath (
    with pkgs;
    [
      apparmor-parser
      bash
      coreutils
      gawk
      gnugrep
      gnused
      perl
      util-linux
      zstd
    ]
  );
in
stdenvNoCC.mkDerivation {
  pname = "apparmor-kernel-regression";
  version = "5.0.2";

  buildCommand = ''
    mkdir -p $out/upstream
    cp -a ${pkgs.apparmor-parser.src}/tests/regression/apparmor/. $out/upstream/
    cp ${pkgs.apparmor-parser.src}/LICENSE $out/LICENSE
    cp ${pkgs.apparmor-parser.src}/parser/parser.conf $out/parser.conf
    cp ${conformanceSrc}/apparmor/run-e2e.sh $out/run-e2e.sh
    substitute ${conformanceSrc}/apparmor/run_apparmor.sh $out/run_apparmor.sh \
      --replace-fail @runtimePath@ '${runtimePath}' \
      --replace-fail @bash@ '${pkgs.bash}/bin/bash'
    chmod +x $out/run_apparmor.sh $out/run-e2e.sh
  '';
}
