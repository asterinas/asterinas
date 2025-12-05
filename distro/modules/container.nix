{ config, lib, pkgs, ... }: {
  virtualisation.containers.storage.settings = {
    storage = {
      driver = "vfs";
      runroot = "/run/containers/storage";
      graphroot = "/var/lib/containers/storage";
    };
  };

  virtualisation.containers.policy = {
    default = [{ type = "insecureAcceptAnything"; }];
  };

  virtualisation.containers.containersConf.settings = {
    containers = {
      cgroupns = "host";
      cgroups = "enabled";
      default_sysctls = [ ];
      ipcns = "host";
      keyring = false;
      netns = "host";
      pidns = "host";
      privileged = true;
      seccomp_profile = "unconfined";
      userns = "host";
      devices = [ ];
    };

    secrets = { };
    network = { };
    engine = {
      cgroup_manager = "cgroupfs";
      events_logger = "none";
      no_pivot_root = true;
      runtime = "runc";
    };
  };
}
