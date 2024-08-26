{ lib
, pkgs
, stdenv
, buildBazelPackage
, fetchFromGitHub
, bazel_7
, libbpf, glibc_multi
, git, go, llvmPackages
}:

buildBazelPackage rec {
  pname = "syscall-tests";
  version = "20240527.0";

  src = fetchFromGitHub {
    owner = "google";
    repo = "gvisor";
    rev = "release-${version}";
    sha256 = "sha256-kUHNEZ5nVc6hrJEo0umT7c97hkd9u2D9rYBbqdscxHU=";
  };

  patches = [ ./gvisor-use_host_go.patch ];

  buildInputs = [ libbpf glibc_multi ];
  nativeBuildInputs = [ git go llvmPackages.clang ];

  # target "bpf" doesn't support "zero-call-used-regs" option
  hardeningDisable = [ "zerocallusedregs" ];

  # Enable "enableNixHacks" to match `buildBazelPackage`, so we won't build two bazels
  bazel = (bazel_7.override { enableNixHacks = true; });

  bazelTargets = [ "//test/syscalls/..." ];
  bazelBuildFlags = [ "--test_tag_filters=native" ];
  removeRulesCC = false;
  removeLocalConfigCc = false;
  removeLocal = false;

  NIX_CC_AARCH64 = "${pkgs.pkgsCross.aarch64-multiplatform.stdenv.cc}/bin/aarch64-unknown-linux-gnu-";

  preBuild = ''
    patchShebangs .

    cp ${./crosstool.patch} tools/crosstool-arm-dirs.patch
  '';

  fetchAttrs = {
    preInstall = ''
      rm -rf $bazelOut/external/{go_sdk,\@go_sdk.marker}
      rm -rf $bazelOut/external/{bazel_gazelle_go_repository_tools,\@bazel_gazelle_go_repository_tools.marker}
      chmod -R +w $bazelOut/external/bazel_gazelle_go_repository_cache
      rm -rf $bazelOut/external/{bazel_gazelle_go_repository_cache,\@bazel_gazelle_go_repository_cache.marker}
      rm -f "$bazelOut"/java.log "$bazelOut"/java.log.*
    '';

    sha256 = "sha256-xQ5lJX4eXGL5CfsxZa8z16dIsT90oZp/p63QLKO1QXg=";
  };

  buildAttrs = {
    preBuild = preBuild + ''
      # fix stack_chk_fail linking error, as freestanding mode should not use stack protector?
      sed -i '35i "-fno-stack-protector " +' test/syscalls/linux/rseq/BUILD

      # disable stack protector for BPF binaries, and ignore warnings caused by '--gcc-toolchain'
      substituteInPlace tools/bazeldefs/defs.bzl \
        --replace-fail ' -Werror' ' -fno-stack-protector'

      # fix: openat with O_CREAT or O_TMPFILE in third argument needs 4 arguments
      substituteInPlace test/syscalls/linux/sticky.cc \
        --replace-fail 'openat(parent_fd.get(), "file", O_CREAT)' 'openat(parent_fd.get(), "file", O_CREAT, 0644)'
    '';

    installPhase = ''
      mkdir -p $out/bin
      cp ./bazel-bin/test/syscalls/linux/*_test $out/bin
    '';
  };
}
