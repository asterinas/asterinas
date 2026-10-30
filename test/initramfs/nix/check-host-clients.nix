# SPDX-License-Identifier: MPL-2.0
#
# Host clients must run on the build machine and remain independent of the guest.
# Regression test for https://github.com/asterinas/asterinas/issues/4012.
{ system }:
let
  clients = [
    "apacheHttpd"
    "iperf3"
    "libmemcached"
    "lmbench"
    "redis"
  ];
  # Match `make install_host_pkgs`, which does not pass a guest target.
  installed = import ./. { inherit system; };
  guestPackages = map (target: import ./. { inherit system target; }) [
    "x86_64"
    "riscv64"
    "aarch64"
  ];
  checkClient =
    name:
    let
      drv = installed.${name};
      builtFor = drv.stdenv.hostPlatform.system;
    in
    if builtFor != system then
      throw "`${name}` is built for ${builtFor}, not ${system}"
    else if !builtins.all (packages: packages.${name}.drvPath == drv.drvPath) guestPackages then
      throw "`${name}` changes with the guest target on ${system}"
    else
      drv.drvPath;
in
map checkClient clients
