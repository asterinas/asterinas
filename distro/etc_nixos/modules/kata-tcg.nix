# SPDX-License-Identifier: MPL-2.0

{ config, lib, pkgs, ... }:

let
  kataRuntime = pkgs.kata-runtime-tcg;
  kataImages = pkgs.kata-runtime.passthru.kata-images;
  guestImage = "/var/lib/kata-containers/${builtins.baseNameOf kataImages}.img";
  overrides = pkgs.writeText "kata-tcg-overrides.json" (builtins.toJSON {
    hypervisor.qemu = {
      image = guestImage;
      default_vcpus = 1;
      default_maxvcpus = 1;
      default_memory = 256;
      default_maxmemory = 256;
      disable_image_nvdimm = true;
      block_device_driver = "virtio-blk";
      disable_vhost_net = true;
      virtio_fs_extra_args = [
        "--thread-pool-size=1"
        "--announce-submounts"
        "--sandbox=none"
        "--seccomp=none"
        "--inode-file-handles=never"
      ];
    };
    agent.kata.dial_timeout = 180;
    runtime = {
      internetworking_model = "none";
      disable_new_netns = true;
      sandbox_cgroup_only = true;
    };
  });
  python = pkgs.python3.withPackages (ps: [ ps.tomli-w ]);
  kataConfiguration = pkgs.runCommand "kata-tcg.toml" { } ''
    ${python}/bin/python ${./kata-tcg-config.py} \
      ${kataRuntime}/share/defaults/kata-containers/configuration-qemu.toml \
      ${overrides} "$out"
  '';
in {
  options.aster_nixos.kata-tcg.enable = lib.mkEnableOption ''
    experimental containerd and Kata with QEMU TCG for trusted x86-64 images.
    Container networking and CRI are disabled. Host cgroup enforcement and
    virtiofsd's namespace sandbox and seccomp filter are unavailable
  '';

  config = lib.mkIf config.aster_nixos.kata-tcg.enable {
    assertions = [{
      assertion = pkgs.stdenv.hostPlatform.system == "x86_64-linux";
      message = "The Kata TCG configuration supports x86_64-linux only.";
    }];

    nixpkgs.overlays = [ (import ../overlays/kata-tcg/default.nix) ];

    virtualisation.containerd = {
      enable = true;
      settings = {
        version = 2;
        # Keep the internal CRI image/runtime plugins used by other services.
        disabled_plugins = [ "io.containerd.grpc.v1.cri" ];
      };
    };

    environment.systemPackages = [ kataRuntime ];
    environment.etc."kata-containers/configuration.toml".source =
      kataConfiguration;

    # Kata 3.29's vendored shim helper creates this directory with mode 0600.
    # The parent needs traversal permission; the socket remains private.
    systemd.tmpfiles.rules = [ "d /run/containerd/s 0700 root root -" ];

    systemd.services.containerd = {
      environment.KATA_CONF_FILE = "/etc/kata-containers/configuration.toml";
      path = [ kataRuntime ];
      # QEMU opens the guest image for writing, outside the immutable Nix store.
      preStart = lib.mkBefore ''
        install -d -m 0755 /var/lib/kata-containers
        if [ ! -e ${guestImage} ]; then
          install -m 0644 \
            ${kataImages}/share/kata-containers/kata-containers.img \
            ${guestImage}.tmp
          mv ${guestImage}.tmp ${guestImage}
        fi
      '';
    };
  };
}
