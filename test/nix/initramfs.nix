{ lib, stdenv, fetchFromGitHub, hostPlatform, writeClosure, busybox, apps
, linux_vdso, benchmark, syscall, }:
let
  etc = lib.fileset.toSource {
    root = ./../src/etc;
    fileset = ./../src/etc;
  };
  gvisor_libs = builtins.path {
    name = "gvisor-libs";
    path = "/lib/x86_64-linux-gnu";
  };
  all_pkgs = [ apps busybox etc linux_vdso ]
    ++ lib.optionals (benchmark != null) [ benchmark.package ]
    ++ lib.optionals (syscall != null) [ syscall.package ];
in stdenv.mkDerivation {
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
    ${lib.optionalString hostPlatform.isx86_64 ''
      cp -r ${linux_vdso}/vdso64.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    ''}
    ${lib.optionalString hostPlatform.isRiscV64 ''
      cp -r ${linux_vdso}/riscv64-vdso.so $out/usr/lib/x86_64-linux-gnu/vdso64.so
    ''}

    cp -r ${apps}/* $out/test/

    cp -r ${etc}/* $out/etc/

    ${lib.optionalString (benchmark != null) ''
      cp -r "${benchmark.package}"/* $out/benchmark/
    ''}

    ${lib.optionalString (syscall != null) ''
      cp -r "${syscall.package}"/opt/* $out/opt/

      # FIXME: Build gvisor syscall test with nix to avoid manual library copying.
      if [ "${syscall.testSuite}" == "gvisor" ]; then
        cp -L ${gvisor_libs}/ld-linux-x86-64.so.2 $out/lib64/ld-linux-x86-64.so.2
        cp -L ${gvisor_libs}/libstdc++.so.6 $out/lib/x86_64-linux-gnu/libstdc++.so.6
        cp -L ${gvisor_libs}/libgcc_s.so.1 $out/lib/x86_64-linux-gnu/libgcc_s.so.1
        cp -L ${gvisor_libs}/libc.so.6 $out/lib/x86_64-linux-gnu/libc.so.6
        cp -L ${gvisor_libs}/libm.so.6 $out/lib/x86_64-linux-gnu/libm.so.6
      fi
    ''}

    # Use `writeClosure` to retrieve all dependencies of the specified packages.
    # This will generate a text file containing the complete closure of the packages,
    # including the packages themselves.
    # The output of `writeClosure` is equivalent to `nix-store -q --requisites`.
    pkg_path=${lib.strings.concatStringsSep ":" all_pkgs}
    while IFS= read -r dep_path; do
      if [[ "$pkg_path" == *"$dep_path"* ]]; then
        continue
      fi
      mkdir -p $out/$dep_path
      cp -r $dep_path/* $out/$dep_path/
    done < ${writeClosure all_pkgs}
  '';
}
