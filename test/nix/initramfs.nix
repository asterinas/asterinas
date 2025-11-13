{ lib, stdenvNoCC, fetchFromGitHub, hostPlatform, writeClosure, busybox, apps
, benchmark, syscall, dnsServer, pkgs, cacert, podman ? null }:
let
  etc = lib.fileset.toSource {
    root = ./../src/etc;
    fileset = ./../src/etc;
  };
  podman_config_files = lib.fileset.toSource {
    root = ./podman/etc;
    fileset = ./podman/etc;
  };
  gvisor_libs = builtins.path {
    name = "gvisor-libs";
    path = "/lib/x86_64-linux-gnu";
  };
  resolv_conf = pkgs.callPackage ./resolv_conf.nix { dnsServer = dnsServer; };
  all_pkgs = [ busybox etc resolv_conf ]
    ++ lib.optionals (apps != null) [ apps.package ]
    ++ lib.optionals (benchmark != null) [ benchmark.package ]
    ++ lib.optionals (syscall != null) [ syscall.package ]
    ++ lib.optionals (podman != null) [ podman cacert ];
in stdenvNoCC.mkDerivation {
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

    cp -r ${etc}/* $out/etc/

    cp ${resolv_conf}/resolv.conf $out/etc/

    ${lib.optionalString (apps != null) ''
      cp -r ${apps.package}/* $out/test/
    ''}

    ${lib.optionalString (benchmark != null) ''
      cp -r "${benchmark.package}"/* $out/benchmark/
    ''}

    ${lib.optionalString (syscall != null) ''
      cp -r "${syscall.package}"/opt/* $out/opt/

      # FIXME: Build gvisor syscall test with nix to avoid manual library copying.
      if [ "${syscall.testSuite}" == "gvisor" ]; then
        mkdir -p $out/lib/x86_64-linux-gnu
        cp -L ${gvisor_libs}/ld-linux-x86-64.so.2 $out/lib64/ld-linux-x86-64.so.2
        cp -L ${gvisor_libs}/libstdc++.so.6 $out/lib/x86_64-linux-gnu/libstdc++.so.6
        cp -L ${gvisor_libs}/libgcc_s.so.1 $out/lib/x86_64-linux-gnu/libgcc_s.so.1
        cp -L ${gvisor_libs}/libc.so.6 $out/lib/x86_64-linux-gnu/libc.so.6
        cp -L ${gvisor_libs}/libm.so.6 $out/lib/x86_64-linux-gnu/libm.so.6
      fi
    ''}

    ${lib.optionalString (podman != null) ''
      mkdir -p $out/{lib,libexec,share}
      cp -r ${podman}/bin/* $out/bin
      cp -r ${podman}/lib/* $out/lib
      cp -r ${podman}/libexec/* $out/libexec
      cp -r ${podman}/share/* $out/share
      mkdir -p $out/var/tmp
      mkdir -p $out/usr/lib/x86_64-linux-gnu
      mkdir -p $out/etc
      cp -r ${podman_config_files}/* $out/etc/
      mkdir -p $out/nix/store
      cp -r ${podman} $out/nix/store/

      mkdir -p $out/etc/ssl/certs
      cp -r ${cacert}/etc/ssl/certs/ca-bundle.crt $out/etc/ssl/certs/ca-certificates.crt
    ''}

    # Use `writeClosure` to retrieve all dependencies of the specified packages.
    # This will generate a text file containing the complete closure of the packages,
    # including the packages themselves.
    # The output of `writeClosure` is equivalent to `nix-store -q --requisites`.
    mkdir -p $out/nix/store
    pkg_path=${lib.strings.concatStringsSep ":" all_pkgs}
    while IFS= read -r dep_path; do
      if [[ "$pkg_path" == *"$dep_path"* ]]; then
        continue
      fi
      cp -r $dep_path $out/nix/store/
    done < ${writeClosure all_pkgs}
  '';
}
