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
  lib = pkgs.lib;

  apps = pkgs.callPackage ./apps.nix { inherit pkgs; };
  busybox = pkgs.busybox;
  benchmark = if enableBenchmark == "true" then
    pkgs.callPackage ./benchmark.nix { inherit pkgs; }
  else
    "";
  syscall_test = if enableSyscallTest == "true" then
    pkgs.callPackage ./syscall_test.nix { inherit pkgs syscallTestWorkDir; }
  else
    "";
  etc = pkgs.lib.fileset.toSource {
    root = ./..;
    fileset = ../etc;
  };
  vdso = pkgs.fetchFromGitHub {
    owner = "asterinas";
    repo = "linux_vdso";
    rev = "be255018febf8b9e2d36f356f6aeb15896521618";
    hash = "sha256-F5RPtu/Hh2hDnjm6/0mc0wGqhQtfMNvPP+6/Id9Hcpk";
  };
  all_pkgs = [ apps busybox etc vdso benchmark syscall_test ];
in pkgs.stdenv.mkDerivation {
  name = "initramfs";
  buildCommand = ''
    mkdir -p $out/{dev,etc,root,usr,opt,tmp,var,proc,sys}
    mkdir -p $out/{benchmark,test,ext2,exfat}
    mkdir -p $out/usr/{bin,sbin,lib,lib64,local}
    ln -sfn usr/bin $out/bin
    ln -sfn usr/sbin $out/sbin
    ln -sfn usr/lib $out/lib
    ln -sfn usr/lib64 $out/lib64
    cp -r ${busybox}/bin/* $out/bin/

    mkdir -p $out/usr/lib/x86_64-linux-gnu
    if [ ${target} == "x86_64" ]; then
      cp -r ${vdso}/vdso64.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    elif [ ${target} == "riscv64" ]; then
      cp -r ${vdso}/riscv64-vdso.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    fi

    cp -r ${apps}/* $out/

    cp -r ${etc}/* $out/

    if [ "${enableSyscallTest}" == "true" ]; then
      cp -r ${syscall_test}/* $out/
    fi

    if [ "${enableBenchmark}" == "true" ]; then
      cp -r ${benchmark}/* $out/
    fi

    pkg_path=${lib.strings.concatStringsSep ":" all_pkgs}
    while IFS= read -r dep_path; do
      if [[ "$pkg_path" == *"$dep_path"* ]]; then
        continue
      fi
      mkdir -p $out/$dep_path
      cp -r $dep_path/* $out/$dep_path/
    done < ${pkgs.writeClosure (lib.lists.filter (p: p != "") all_pkgs)}
  '';
}
