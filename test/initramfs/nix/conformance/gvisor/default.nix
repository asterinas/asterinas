{
  lib,
  pkgs,
  callPackage,
  applyPatches,
  fetchFromGitHub,
  stdenv,
}:
let
  # //vdso:vdso builds an AArch64 vDSO even when the syscall tests are built
  # for x86_64, so the vendored Coral toolchain needs both compilers.
  aarch64CC = pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc;
  aarch64Config = pkgs.pkgsCross.aarch64-multiplatform.stdenv.hostPlatform.config;
  buildAutoPatchelfPath = lib.makeBinPath [
    pkgs.buildPackages."auto-patchelf"
    pkgs.buildPackages.patchelf
    pkgs.buildPackages.stdenv.cc.bintools
  ];
  buildAutoPatchelfHook = pkgs.buildPackages.writeTextFile {
    name = "gvisor-build-auto-patchelf-hook";
    destination = "/nix-support/setup-hook";
    text = ''
      export PATH="${buildAutoPatchelfPath}:$PATH"
      export NIX_BINTOOLS=${pkgs.buildPackages.stdenv.cc.bintools}
      source ${pkgs.buildPackages.autoPatchelfHook}/nix-support/setup-hook
    '';
  };
  callBazelDerivation =
    path: args: bazelArgs:
    (callPackage path (args // { lndir = pkgs.buildPackages.lndir; })) (
      bazelArgs
      // lib.optionalAttrs (bazelArgs.command == "vendor") {
        impureEnvVars = (bazelArgs.impureEnvVars or [ ]) ++ lib.fetchers.proxyImpureEnvVars;
      }
    );
  bazelPackage = callPackage (pkgs.path + "/pkgs/by-name/ba/bazel_8/build-support/bazelPackage.nix") {
    # The helper adds this hook to its vendor post-processing derivation.
    # It must run on the build platform when cross-compiling.
    autoPatchelfHook = buildAutoPatchelfHook;
    # The helper does not forward extra bazelVendorDepsFOD attributes, so pass
    # proxy settings to the fixed-output vendor derivation here.
    callPackage = callBazelDerivation;
    # The helper runs `grep` on the build machine to clean up vendored dependencies.
    gnugrep = pkgs.buildPackages.gnugrep;
  };

  # Bazel runs on the build platform while producing target-platform binaries.
  buildBazel = pkgs.buildPackages.bazel_8;
  # Bazel derives its default output base from the randomized Nix build path.
  # Gazelle embeds that path in generated tools and repository markers, so use
  # a clean, fixed output base for the fixed-output vendor derivation only.
  bazelWithStableVendorEnv = pkgs.buildPackages.writeShellScriptBin "bazel" ''
    for arg in "$@"; do
      if [[ "$arg" == vendor ]]; then
        cleanup_vendor_dirs() {
          rm -rf \
            /tmp/gvisor-bazel-home \
            /tmp/gvisor-bazel-output \
            /tmp/gvisor-bazel-tmp
        }
        trap cleanup_vendor_dirs EXIT
        cleanup_vendor_dirs
        mkdir -p /tmp/gvisor-bazel-home /tmp/gvisor-bazel-tmp
        HOME=/tmp/gvisor-bazel-home \
        SOURCE_DATE_EPOCH=315532800 \
        TMPDIR=/tmp/gvisor-bazel-tmp \
        USE_BAZEL_VERSION=${buildBazel.version} \
        ${buildBazel}/bin/bazel \
          --output_base=/tmp/gvisor-bazel-output \
          "$@"
        status=$?
        if [[ "$status" -eq 0 ]]; then
          # Drop caches and links back into Bazel's temporary output tree.
          rm -rf \
            vendor_dir/bazel-external \
            vendor_dir/gazelle++non_module_deps+bazel_gazelle_go_repository_tools/src/github.com/bazelbuild/bazel-gazelle \
            vendor_dir/gazelle++non_module_deps+bazel_gazelle_go_repository_cache/gocache
        fi
        exit "$status"
      fi
    done
    exec ${buildBazel}/bin/bazel "$@"
  '';
  registry = fetchFromGitHub {
    owner = "bazelbuild";
    repo = "bazel-central-registry";
    rev = "eb05b77f41810ea7fa811244050b588675e8cd95";
    hash = "sha256-ed9q8dKRjZPPvmpA7HrQiQ7iMdHrkRFkDEi/orgNO74=";
  };
  package = bazelPackage rec {
    name = "gvisor-syscall-tests-${version}";
    version = "20260622.0";

    src = applyPatches {
      src = fetchFromGitHub {
        owner = "google";
        repo = "gvisor";
        rev = "release-${version}";
        hash = "sha256-VjKn1ACNhiNsPgXEvekf54ZcsPL3Xq5Rn21rSGVTfC0=";
      };
      patches = [ ./0001-gvisor-Provide-SDK-metadata-required-by-Bazel-8.6.patch ];
      postPatch = ''
        cp ${./0002-rules_go-Handle-immutable-facts-for-Bazel-8.6.patch} \
          tools/rules_go_bazel_8_6.patch
      '';
    };

    bazel = bazelWithStableVendorEnv;
    inherit registry;
    # Build every syscall test binary, but exclude the Go-based runner in the
    # parent //test/syscalls package.
    targets = [ "//test/syscalls/linux:all" ];
    # This package currently supports x86_64 test binaries only. Pin Bazel's
    # target independently of the platform on which Bazel itself runs.
    commandArgs =
      if stdenv.hostPlatform.isx86_64 then
        [
          "--config=x86_64"
          # Nixpkgs GCC needs this to scrub header paths from diagnostic strings.
          "--action_env=NIX_STORE"
        ]
      else
        throw "gVisor syscall tests currently support only x86_64 targets";
    # Nixpkgs installs Bazel through its official version-selecting wrapper.
    # Override gVisor's .bazelversion (8.3.1) with the packaged Bazel 8.6.
    env.USE_BAZEL_VERSION = buildBazel.version;

    # Vendor hashes are specific to the build platform. Revalidate both when
    # dependencies or vendoring logic change.
    bazelVendorDepsFOD = {
      outputHash =
        {
          x86_64-linux = "sha256-TSTCm4o8tkwcBhZ7ne+jUrY5Mivbw1qM1dSEe9HtXPU=";
          aarch64-linux = "sha256-7OYnylERa22/ugh68XX4vlZsw737+x2cmVqxQnHX0Ew=";
        }
        .${stdenv.buildPlatform.system};
      outputHashAlgo = "sha256";
    };

    # These libraries are referenced only by Go's debug/elf test fixtures.
    autoPatchelfIgnoreMissingDeps = [
      "libstdc++.so.6"
      "libtiff.so.6"
    ];

    installPhase = ''
      runHook preInstall

      install -Dm555 -t "$out" bazel-bin/test/syscalls/linux/*_test

      runHook postInstall
    '';
  };
in
package.overrideAttrs (oldAttrs: {
  postPatch = (oldAttrs.postPatch or "") + ''
    patchShebangs tools/workspace_status.sh
  '';

  preBuildPhase = oldAttrs.preBuildPhase + ''
    crosstoolFile=vendor_dir/+coral_crosstool_extension+coral_crosstool/cc_toolchain_config.bzl.tpl
    cp --dereference "$crosstoolFile" "$crosstoolFile.tmp"
    chmod u+w "$crosstoolFile.tmp"
    mv "$crosstoolFile.tmp" "$crosstoolFile"

    patch -d vendor_dir -p1 < ${./0003-crosstool-Use-Nix-tool-paths-for-sandbox-builds.patch}
    substituteInPlace "$crosstoolFile" \
      --replace-fail '@NIX_CC_WRAPPER@' '${stdenv.cc}' \
      --replace-fail '@NIX_GCC@' '${stdenv.cc.cc}' \
      --replace-fail '@NIX_GCC_VERSION@' '${stdenv.cc.cc.version}' \
      --replace-fail '@NIX_TARGET_CONFIG@' '${stdenv.hostPlatform.config}' \
      --replace-fail '@NIX_TARGET_PREFIX@' '${stdenv.cc.targetPrefix}' \
      --replace-fail '@NIX_LIBC_DEV@' '${stdenv.cc.libc.dev}' \
      --replace-fail '@NIX_AARCH64_CC_WRAPPER@' '${aarch64CC}' \
      --replace-fail '@NIX_AARCH64_GCC@' '${aarch64CC.cc}' \
      --replace-fail '@NIX_AARCH64_GCC_VERSION@' '${aarch64CC.cc.version}' \
      --replace-fail '@NIX_AARCH64_TARGET_CONFIG@' '${aarch64Config}' \
      --replace-fail '@NIX_AARCH64_TARGET_PREFIX@' '${aarch64CC.targetPrefix}' \
      --replace-fail '@NIX_AARCH64_LIBC_DEV@' '${aarch64CC.libc.dev}'
  '';
})
