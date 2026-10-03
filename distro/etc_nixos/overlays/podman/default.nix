final: prev: {
  runc = prev.runc.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ]) ++ [
      ./runc-Disable-container-state-check.patch
      ./runc-Disable-creating-dev-mqueue.patch
      ./runc-Disable-eBPF-for-device-filtering.patch
    ];
  });
  podman =
    (prev.podman.overrideAttrs (oldAttrs: {
      patches = (oldAttrs.patches or [ ]) ++ [
        ./Podman-Disable-etc-hosts-and-etc-resolv-conf-injection.patch
      ];
    })).override
      { runc = final.runc; };
}
