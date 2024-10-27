{ lib
, system
, busybox
, sysbench, iperf, unixbench, iozone, fio
, membench, lmbench, test-apps, gvisor-syscall-tests
, makeInitrdNG, buildEnv, callPackage
, fetchurl
, buildPackages
}:
let
  test-apps = callPackage ./apps/default.nix { };
in buildPackages.makeInitrdNG {
  name = "aster-initrd";
  compressor = "gzip";

  contents = let
    initrdBinEnv = buildEnv {
      name = "initrd-bin-env";
      paths = [
        busybox
      ];
      pathsToLink = ["/bin" "/sbin"];
    };

    benchmarkBinEnv = buildEnv {
      name = "benchmark-bin-env";
      paths = [
        sysbench
        membench
        lmbench
        iperf
        unixbench
        iozone
        fio
      ];
    };

    vdso = fetchurl {
      url = "https://raw.githubusercontent.com/asterinas/linux_vdso/2a6d2db/vdso64.so";
      sha256 = "sha256-J8179XapL6SKiRwFmI9S+sNbc3TVuWUNawNeR3xdk6M=";
    };

    contents = {
      # Required Mountpoints
      "/dev/.keep".text = "/dev mount point";
      "/proc/.keep".text = "/proc mount point";
      "/ext2/.keep".text = "/ext2 mount point";
      "/exfat/.keep".text = "/exfat mount point";

      # Required Directories and Files
      "/tmp/.keep".text = "/tmp directory";
      "/usr/bin/busybox".source = "${initrdBinEnv}/bin/busybox";

      "/bin".source = "${initrdBinEnv}/bin";
      "/sbin".source = "${initrdBinEnv}/sbin";
      "/etc".source = ./etc;
      "/lib/x86_64-linux-gnu/vdso64.so".source = vdso;

    } // lib.optionalAttrs (system == "x86_64-linux") {
      # Test Files
      "/opt/syscall_test".source = gvisor-syscall-tests;
      "/benchmark/bin".source = "${benchmarkBinEnv}/bin";
      "/benchmark/benchmarks".source = ./benchmark;
      "/test".source = test-apps;

    } // lib.optionalAttrs (system == "riscv64-linux") {
      "/test/boot_hello.sh".source = ./apps/scripts/boot_hello.sh;
    };

    storePaths = [
    ];

    handleContent = symlink: v: if builtins.hasAttr "text" v
      then builtins.toFile (builtins.baseNameOf symlink) v.text
      else v.source;
  in
    map (path: { object = path; symlink = null; }) storePaths
    ++ lib.mapAttrsToList (k: v: { object = handleContent k v; symlink = k; }) contents;
}
