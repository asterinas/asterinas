final: prev: {
  runc = prev.runc.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ]) ++ [
      ./runc-Disable-container-state-check.patch
      ./runc-Disable-creating-dev-mqueue.patch
      ./runc-Disable-eBPF-for-device-filtering.patch
      ./runc-Disable-user-and-capability-setup-checks.patch
      ./runc-Switch-MS_SLAVE-to-MS_PRIVATE.patch
    ];
  });
  podman = (prev.podman.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ])
      ++ [ ./Podman-Disable-etc-hosts-and-etc-resolv-conf-injection.patch ];
  })).override { runc = final.runc; };
}
