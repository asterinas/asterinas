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
  native_pkgs = import <nixpkgs> { };
in pkgs.stdenv.mkDerivation {
  name = "initramfs";
  nativeBuildInputs = [ native_pkgs.binutils ];
  buildCommand = ''
    resolve_libs() {
      while IFS= read -r lib_path; do
        if [ ! -d "$lib_path" ]; then
          continue
        fi
        if [ -d "$out/$lib_path" ]; then
          continue
        fi
        mkdir -p $out/$lib_path
        cp -r $lib_path/* $out/$lib_path/
      done <<< "$(patchelf --print-rpath "$1" | tr ":" "\n")"
    }

    mkdir -p $out/{dev,etc,root,usr,opt,tmp,var,proc,sys}
    mkdir -p $out/{benchmark,test,ext2,exfat}
    mkdir -p $out/usr/{bin,sbin,lib,lib64,local}
    ln -sfn usr/bin $out/bin
    ln -sfn usr/sbin $out/sbin
    ln -sfn usr/lib $out/lib
    ln -sfn usr/lib64 $out/lib64
    cp -r ${pkgs.busybox}/bin/* $out/bin/
    resolve_libs $out/bin/busybox

    mkdir -p $out/usr/lib/x86_64-linux-gnu
    if [ ${target} == "x86_64" ]; then
      cp -r ${vdso}/vdso64.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    elif [ ${target} == "riscv64" ]; then
      cp -r ${vdso}/riscv64-vdso.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    fi

    cp -r ${etc}/* $out/

    cp -r ${apps}/* $out/
    resolve_libs $out/test/network/tcp_server

    if [ "${syscall_test}" ]; then
      cp -r ${syscall_test}/* $out/
    fi

    if [ "${benchmark}" ]; then
      cp -r ${benchmark}/* $out/

      resolve_libs $out/benchmark/bin/fio
      resolve_libs $out/benchmark/bin/iperf3
      resolve_libs $out/benchmark/bin/lmbench/lmdd
      resolve_libs $out/benchmark/bin/memcached
      resolve_libs $out/benchmark/bin/nginx
      resolve_libs $out/benchmark/bin/redis-server
      resolve_libs $out/benchmark/bin/sysbench

      mkdir -p $out/var/log/nginx

      mkdir -p $out/${pkgs.krb5.lib}
      cp -r ${pkgs.krb5.lib}/lib $out/${pkgs.krb5.lib}/

      mkdir -p $out/${pkgs.keyutils.lib}/
      cp -r ${pkgs.keyutils.lib}/lib $out/${pkgs.keyutils.lib}/

      mkdir -p $out/${pkgs.libcap.lib}/
      cp -r ${pkgs.libcap.lib}/lib $out/${pkgs.libcap.lib}/

      libgcc_realpath=$(readlink -f ${pkgs.gcc-unwrapped.lib}/lib/libgcc_s.so.1)
      libgcc_dir=$(dirname "$libgcc_realpath")
      mkdir -p $out/$libgcc_dir
      cp -r $libgcc_dir/* $out/$libgcc_dir/
    fi
  '';
}
