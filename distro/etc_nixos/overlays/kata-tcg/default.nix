# SPDX-License-Identifier: MPL-2.0

final: prev: {
  # A separate package confines these experimental changes to the TCG module.
  # The patches target the pinned Nixpkgs package, Kata Containers 3.29.0.
  kata-runtime-tcg = prev.kata-runtime.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [
      ./0001-Use-QEMU-TCG-without-hypervisor-device-discovery.patch
      ./0002-Disable-host-syslog-delivery.patch
      ./0003-Continue-after-host-cgroup-placement-failure.patch
    ];
  });
}
