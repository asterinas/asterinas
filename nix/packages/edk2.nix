# SPDX-License-Identifier: MIT
#
# Vendored from nixpkgs pkgs/by-name/ed/edk2/package.nix (MIT-licensed)
# and pinned to the edk2 tag used by osdk/tools/docker/Dockerfile.
#
# Local changes from nixpkgs:
# - use edk2-stable202508 and its matching source hash;
# - reapply the antlr/dlg cross-build fix below because nixpkgs' patch no
#   longer matches the 202508 VfrCompile makefile;
# - keep edk2's OpenSSL submodule, which this tag's CryptoPkg references;
# - set PYTHON_COMMAND so BaseTools does not probe /usr/bin/env python3;
# - drop nixpkgs' updateScript, which would bump the pin to the latest tag.
#
# Replacing `edk2` in the overlay also makes pkgsCross.gnu64.{edk2,OVMF}
# build from this pinned source.
#
{ stdenv, fetchFromGitHub, fetchpatch, applyPatches, libuuid, bc, lib
, buildPackages, nixosTests, }:

let
  pythonEnv = buildPackages.python3.withPackages (ps: [ ps.tkinter ]);

  targetArch = if stdenv.hostPlatform.isi686 then
    "IA32"
  else if stdenv.hostPlatform.isx86_64 then
    "X64"
  else if stdenv.hostPlatform.isAarch32 then
    "ARM"
  else if stdenv.hostPlatform.isAarch64 then
    "AARCH64"
  else if stdenv.hostPlatform.isRiscV64 then
    "RISCV64"
  else if stdenv.hostPlatform.isLoongArch64 then
    "LOONGARCH64"
  else
    throw "Unsupported architecture";

  buildType = if stdenv.hostPlatform.isDarwin then "CLANGPDB" else "GCC5";

  edk2 = stdenv.mkDerivation {
    pname = "edk2";
    version = "202508";

    srcWithVendoring = fetchFromGitHub {
      owner = "tianocore";
      repo = "edk2";
      rev = "edk2-stable${edk2.version}";
      fetchSubmodules = true;
      hash = "sha256-YZcjPGPkUQ9CeJS9JxdHBmpdHsAj7T0ifSZWZKyNPMk=";
    };

    src = applyPatches {
      name = "edk2-${edk2.version}-unvendored-src";
      src = edk2.srcWithVendoring;

      patches = [
        # Let tools_def.template pick up the cross compiler prefix.
        (fetchpatch {
          url =
            "https://src.fedoraproject.org/rpms/edk2/raw/08f2354cd280b4ce5a7888aa85cf520e042955c3/f/0021-Tweak-the-tools_def-to-support-cross-compiling.patch";
          hash = "sha256-E1/fiFNVx0aB1kOej2DJ2DlBIs9tAAcxoedym2Zhjxw=";
        })
        # nixpkgs' antlr/dlg cross-build patch no longer applies to the
        # reworked 202508 VfrCompile makefile, so keep the edit local.
      ];

      # Keep the OpenSSL submodule that belongs to this edk2 tag. The 202508
      # CryptoPkg .inf files reference files from that tree, so substituting
      # nixpkgs' openssl_3 breaks the firmware build.
      postPatch = ''
        # PCCTS antlr/dlg are host tools. Cross builds must compile them with
        # the build compiler; native builds fall back to CC/CXX.
        substituteInPlace BaseTools/Source/C/VfrCompile/GNUmakefile \
          --replace-fail '$(MAKE) -C Pccts/antlr' '$(MAKE) -C Pccts/antlr CC=$(or $(CC_FOR_BUILD),$(CC)) CXX=$(or $(CXX_FOR_BUILD),$(CXX))' \
          --replace-fail '$(MAKE) -C Pccts/dlg' '$(MAKE) -C Pccts/dlg CC=$(or $(CC_FOR_BUILD),$(CC)) CXX=$(or $(CXX_FOR_BUILD),$(CXX))'

        # Allow BaseTools to compile with Clang.
        # https://bugzilla.tianocore.org/show_bug.cgi?id=4620
        substituteInPlace BaseTools/Conf/tools_def.template --replace-fail \
          'DEFINE CLANGPDB_WARNING_OVERRIDES    = ' \
          'DEFINE CLANGPDB_WARNING_OVERRIDES    = -Wno-unneeded-internal-declaration '
      '';
    };

    nativeBuildInputs = [ pythonEnv ];
    depsBuildBuild = [ buildPackages.stdenv.cc buildPackages.bash ];
    depsHostHost = [ libuuid ];
    strictDeps = true;

    # Same cross-prefix hook used by Fedora's edk2 package:
    # https://src.fedoraproject.org/rpms/edk2/blob/08f2354cd280b4ce5a7888aa85cf520e042955c3/f/edk2.spec#_319
    ${"GCC5_${targetArch}_PREFIX"} = stdenv.cc.targetPrefix;

    # BaseTools probes `/usr/bin/env python3` unless PYTHON_COMMAND is set.
    # That path is absent in sandboxed Nix builds.
    PYTHON_COMMAND = lib.getExe pythonEnv;

    makeFlags = [ "-C BaseTools" ];

    env.NIX_CFLAGS_COMPILE = "-Wno-return-type"
      + lib.optionalString (stdenv.cc.isGNU) " -Wno-error=stringop-truncation"
      + lib.optionalString (stdenv.hostPlatform.isDarwin)
      " -Wno-error=macro-redefined";

    hardeningDisable = [ "format" "fortify" ];

    installPhase = ''
      mkdir -vp $out
      mv -v BaseTools $out
      mv -v edksetup.sh $out
      # patchShebangs does not find these wrappers during cross builds.
      for i in $out/BaseTools/BinWrappers/PosixLike/*; do
        chmod +x "$i"
        patchShebangs --build "$i"
      done
    '';

    enableParallelBuilding = true;

    meta = {
      description = "Intel EFI development kit";
      homepage =
        "https://github.com/tianocore/tianocore.github.io/wiki/EDK-II/";
      changelog =
        "https://github.com/tianocore/edk2/releases/tag/edk2-stable${edk2.version}";
      license = lib.licenses.bsd2;
      platforms = with lib.platforms;
        aarch64 ++ arm ++ i686 ++ x86_64 ++ loongarch64 ++ riscv64;
      maintainers = [ lib.maintainers.mjoerg ];
    };

    passthru = {
      # Keep nixpkgs' channel-blocking smoke test hook.
      tests.uefiUsb = nixosTests.boot.uefiCdrom;

      mkDerivation = projectDscPath: attrsOrFun:
        stdenv.mkDerivation (finalAttrs:
          let attrs = lib.toFunction attrsOrFun finalAttrs;
          in {
            inherit (edk2) src;

            depsBuildBuild = [ buildPackages.stdenv.cc ]
              ++ attrs.depsBuildBuild or [ ];
            nativeBuildInputs = [ bc pythonEnv ]
              ++ attrs.nativeBuildInputs or [ ];
            strictDeps = true;

            ${"GCC5_${targetArch}_PREFIX"} = stdenv.cc.targetPrefix;

            prePatch = ''
              rm -rf BaseTools
              ln -sv ${buildPackages.edk2}/BaseTools BaseTools
            '';

            configurePhase = ''
              runHook preConfigure
              export WORKSPACE="$PWD"
              . ${buildPackages.edk2}/edksetup.sh BaseTools
              runHook postConfigure
            '';

            buildPhase = ''
              runHook preBuild
              build -a ${targetArch} -b ${
                attrs.buildConfig or "RELEASE"
              } -t ${buildType} -p ${projectDscPath} -n $NIX_BUILD_CORES $buildFlags
              runHook postBuild
            '';

            installPhase = ''
              runHook preInstall
              mv -v Build/*/* $out
              runHook postInstall
            '';
          } // removeAttrs attrs [ "nativeBuildInputs" "depsBuildBuild" ]);
    };
  };

in edk2
