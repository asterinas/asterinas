{
  lib,
  stdenv,
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
      python3
      util-linux
      zstd
    ]
  );
  apparmorLib = pkgs.libapparmor.overrideAttrs (_: {
    version = "5.0.2";
    src = pkgs.apparmor-parser.src;
  });
in
stdenv.mkDerivation {
  pname = "apparmor-kernel-regression";
  version = "5.0.2";
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ apparmorLib ];

  buildCommand = ''
    mkdir -p $out/upstream
    cp -a ${pkgs.apparmor-parser.src}/tests/regression/apparmor/. $out/upstream/
    cp ${pkgs.apparmor-parser.src}/LICENSE $out/LICENSE
    cp ${pkgs.apparmor-parser.src}/parser/parser.conf $out/parser.conf
    cp ${conformanceSrc}/apparmor/run-e2e.sh $out/run-e2e.sh
    cp ${conformanceSrc}/apparmor/lifecycle.py $out/lifecycle.py
    $CC -g -O2 $out/upstream/introspect.c \
      $(pkg-config --cflags --libs libapparmor) -o $out/introspect
    substitute ${conformanceSrc}/apparmor/run_apparmor.sh $out/run_apparmor.sh \
      --replace-fail @runtimePath@ '${runtimePath}' \
      --replace-fail @bash@ '${pkgs.bash}/bin/bash'
    chmod +x $out/run_apparmor.sh $out/run-e2e.sh
  '';
}
