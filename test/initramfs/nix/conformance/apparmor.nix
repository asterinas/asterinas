{
  lib,
  stdenv,
  pkgs,
  conformanceSrc,
}:

let
  src = pkgs.apparmor-parser.src;
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
    inherit src;
  });
in
stdenv.mkDerivation {
  pname = "apparmor-kernel-regression";
  version = "5.0.2";
  nativeBuildInputs = [
    pkgs.pkg-config
    pkgs.buildPackages.apparmor-parser
  ];
  buildInputs = [ apparmorLib ];

  buildCommand = ''
    mkdir -p $out/upstream $out/profiles
    cp -a ${src}/tests/regression/apparmor/. $out/upstream/
    cp ${src}/LICENSE $out/LICENSE
    cp ${src}/parser/parser.conf $out/parser.conf

    profiles=${src}/profiles/apparmor.d
    # Keep the upstream feature set but select v5 for serialization.
    sed -E '/^v[6-9] \{yes$/ { N; d; }' "$profiles/abi/5.0" > kernel-features-v5
    for name in curl tinyproxy proftpd; do
      apparmor_parser --config-file="$out/parser.conf" --base="$profiles" \
        --kernel-features="$PWD/kernel-features-v5" -j0 -K -S "$profiles/$name" \
        > "$out/profiles/$name.bin"
    done

    cp ${conformanceSrc}/apparmor/run-e2e.sh $out/run-e2e.sh
    cp ${conformanceSrc}/apparmor/lifecycle.py $out/lifecycle.py
    cp ${conformanceSrc}/apparmor/admission.py $out/admission.py
    $CC -g -O2 $out/upstream/introspect.c \
      $(pkg-config --cflags --libs libapparmor) -o $out/introspect
    substitute ${conformanceSrc}/apparmor/run_apparmor.sh $out/run_apparmor.sh \
      --replace-fail @runtimePath@ '${runtimePath}' \
      --replace-fail @bash@ '${pkgs.bash}/bin/bash'
    chmod +x $out/run_apparmor.sh $out/run-e2e.sh
  '';
}
