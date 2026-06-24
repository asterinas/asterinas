final: prev: {
  runc = prev.runc;
  podman = (prev.podman.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ])
      ++ [ ./Podman-Disable-etc-hosts-and-etc-resolv-conf-injection.patch ];
  })).override { runc = final.runc; };
}
