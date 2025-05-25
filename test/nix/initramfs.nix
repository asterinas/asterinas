{ target ? "x86_64", enableBenchmark ? "false", enableSyscallTest ? "false"
, syscallTestWorkDir ? "/tmp", }:
let
  crossSystem.config = if target == "x86_64" then
    "x86_64-unknown-linux-gnu"
  else if target == "riscv64" then
    "riscv64-unknown-linux-gnu"
  else
    throw "Target arch ${target} not yet supported.";
  pkgs = import <nixpkgs> { inherit crossSystem; };

  apps = pkgs.callPackage ./apps.nix { inherit pkgs; };
  benchmark = if enableBenchmark == "true" then
    pkgs.callPackage ./benchmark.nix { inherit pkgs; }
  else
    "";
  busybox = pkgs.busybox.override { enableStatic = true; };
  etc = pkgs.lib.fileset.toSource {
    root = ./..;
    fileset = ../etc;
  };
  syscall_test = if enableSyscallTest == "true" then
    pkgs.callPackage ./syscall_test.nix { inherit pkgs syscallTestWorkDir; }
  else
    "";
  vdso = pkgs.fetchFromGitHub {
    owner = "asterinas";
    repo = "linux_vdso";
    rev = "be255018febf8b9e2d36f356f6aeb15896521618";
    hash = "sha256-F5RPtu/Hh2hDnjm6/0mc0wGqhQtfMNvPP+6/Id9Hcpk";
  };
in pkgs.stdenv.mkDerivation {
  name = "initramfs";
  buildCommand = ''
    mkdir -m 0755 -p $out/{dev,root,usr,ext2,exfat}
    mkdir -m 0555 -p $out/{proc,sys}
    mkdir -m 1777 -p $out/tmp

    mkdir -p $out/usr/{bin,sbin,lib,lib64,local}
    ln -sfn usr/bin $out/bin
    ln -sfn usr/sbin $out/sbin
    ln -sfn usr/lib $out/lib
    ln -sfn usr/lib64 $out/lib64
    cp -r ${busybox}/bin/* $out/bin/

    mkdir -m 0755 -p $out/etc
    cp -r ${etc}/* $out/

    mkdir -m 0755 -p $out/test
    cp -r ${apps}/* $out/

    if [ "${benchmark}" ]; then
      mkdir -m 0755 -p $out/benchmark
      cp -r ${benchmark}/* $out/
    fi

    if [ "${syscall_test}" ]; then
      mkdir -m 0755 -p $out/opt/syscall_test
      cp -r ${syscall_test}/* $out/
    fi

    mkdir -p $out/usr/lib/x86_64-linux-gnu
    if [ ${target} == "x86_64" ]; then
      cp -r ${vdso}/vdso64.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    elif [ ${target} == "riscv64" ]; then
      cp -r ${vdso}/riscv64-vdso.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    fi

    mkdir -p $out/${pkgs.glibc}/
    cp -r ${pkgs.glibc}/lib $out/${pkgs.glibc}/

    mkdir -p $out/${pkgs.gcc-unwrapped.lib}/lib
    cp -L ${pkgs.gcc-unwrapped.lib}/lib/libgcc_s.so.1 $out/${pkgs.gcc-unwrapped.lib}/lib/
    cp -L ${pkgs.gcc-unwrapped.lib}/lib/libstdc++.so.6 $out/${pkgs.gcc-unwrapped.lib}/lib/

    mkdir -p $out/${pkgs.libnl.out}/
    cp -r ${pkgs.libnl.out}/lib $out/${pkgs.libnl.out}/

    mkdir -p $out/${pkgs.zlib}/
    cp -r ${pkgs.zlib}/lib $out/${pkgs.zlib}/

    mkdir -p $out/${pkgs.libaio}/
    cp -r ${pkgs.libaio}/lib $out/${pkgs.libaio}/

    mkdir -p $out/${pkgs.iperf}/
    cp -r ${pkgs.iperf}/lib $out/${pkgs.iperf}/

    mkdir -p $out/${pkgs.openssl.out}/
    cp -r ${pkgs.openssl.out}/lib $out/${pkgs.openssl.out}/

    mkdir -p $out/${pkgs.lksctp-tools}/
    cp -r ${pkgs.lksctp-tools}/lib $out/${pkgs.lksctp-tools}/

    mkdir -p $out/${pkgs.libtirpc}/
    cp -r ${pkgs.libtirpc}/lib $out/${pkgs.libtirpc}/

    mkdir -p $out/${pkgs.krb5.lib}/
    cp -r ${pkgs.krb5.lib}/lib $out/${pkgs.krb5.lib}/

    mkdir -p $out/${pkgs.keyutils.lib}/
    cp -r ${pkgs.keyutils.lib}/lib $out/${pkgs.keyutils.lib}/

    mkdir -p $out/${pkgs.libmemcached}/
    cp -r ${pkgs.libmemcached}/lib $out/${pkgs.libmemcached}/

    mkdir -p $out/${pkgs.cyrus_sasl.out}/
    cp -r ${pkgs.cyrus_sasl.out}/lib $out/${pkgs.cyrus_sasl.out}/
  '';
}
