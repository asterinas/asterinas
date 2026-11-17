# SPDX-License-Identifier: MPL-2.0

{
  lib,
  stdenvNoCC,
  pkgs,
  conformanceSrc,
  hostPreparation ? false,
}:

let
  erofsRevision = "aaa2ee970b59b1f95533f558037cc5b4be66d01c";

  erofsSource = pkgs.applyPatches {
    name = "erofs-utils-experimental-tests";
    src = pkgs.fetchFromGitHub {
      owner = "erofs";
      repo = "erofs-utils";
      rev = erofsRevision;
      hash = "sha256-XVC506elhyX/6E/zrNzrDnbsIiq7Jg+Qel//A5gwzOs=";
    };
    patches = [
      "${conformanceSrc}/erofs_tests/patches/block-device-adaptations.patch"
      "${conformanceSrc}/erofs_tests/patches/external-fssum.patch"
      "${conformanceSrc}/erofs_tests/patches/host-prepare-phases.patch"
    ];
  };

  erofsUtils = (pkgs.erofs-utils.override { fuseSupport = hostPreparation; }).overrideAttrs (old: {
    version = "1.9.4-g${builtins.substring 0 8 erofsRevision}";
    src = erofsSource;
    postInstall = (old.postInstall or "") + ''
      install -Dm755 tests/src/fssum $out/libexec/fssum
    '';
  });

  runtimeDeps =
    with pkgs;
    [
      erofsUtils
      coreutils
      gnutar
      bash
      gnugrep
      gnused
      gawk
      findutils
      util-linux
      attr
    ]
    ++ lib.optionals hostPreparation [
      pkgs.fuse
    ];

  sbinDeps = with pkgs; [
    util-linux
  ];

  runtimePath =
    lib.makeBinPath runtimeDeps
    + ":"
    + lib.concatMapStringsSep ":" (package: "${package}/sbin") sbinDeps
    + ":/bin:/usr/bin:/sbin:/usr/sbin";
in
stdenvNoCC.mkDerivation {
  name = "erofs-conformance";

  buildCommand = ''
    mkdir -p $out/erofs/upstream
    cp -r ${erofsSource}/. $out/erofs/upstream/
    chmod -R u+w $out/erofs
    find $out/erofs -name '*.orig' -delete

    mkdir -p $out/erofs/bin
    cp -r ${erofsUtils}/bin $out/erofs/
    install -Dm755 ${erofsUtils}/libexec/fssum $out/erofs/bin/fssum

    for script in common.sh phase.sh prepare.sh run_erofs.sh; do
      cp ${conformanceSrc}/erofs_tests/$script $out/erofs/
    done
    sed -i "s|__RUNTIME_PATH__|${runtimePath}|" $out/erofs/common.sh
    chmod +x $out/erofs/prepare.sh $out/erofs/run_erofs.sh
    cp -r ${conformanceSrc}/erofs_tests/run_list $out/erofs/
  '';
}
