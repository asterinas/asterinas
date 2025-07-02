{ stdenv, callPackage, hostPlatform, pkgsHostTarget, }: {
  # Use `--esx` flag to enable `CONFIG_NO_SHM` and disable `CONFIG_HAVE_TIMERFD_CREATE`.
  fio = pkgsHostTarget.fio.overrideAttrs (_: { configureFlags = [ "--esx" ]; });
  hackbench = callPackage ./hackbench.nix { };
  iperf3 = pkgsHostTarget.iperf3;
  lmbench = callPackage ./lmbench.nix { };
  memcached = pkgsHostTarget.memcached;
  nginx = pkgsHostTarget.nginx;
  redis =
    (pkgsHostTarget.redis.overrideAttrs (_: { doCheck = false; })).override {
      withSystemd = false;
    };
  schbench = callPackage ./schbench.nix { };
  sqlite-speedtest1 = callPackage ./sqlite-speedtest1.nix { };
  sysbench = if hostPlatform.isx86_64 then pkgsHostTarget.sysbench else null;
}
