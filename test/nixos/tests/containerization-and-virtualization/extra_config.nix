{ pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ skopeo qemu_kvm ];
  virtualisation.podman.enable = true;

  environment.variables = {
    LINUX_BZIMAGE = "${pkgs.linuxPackages.kernel}/bzImage";
    OVMF_PATH = "${pkgs.OVMF.fd}/FV/OVMF.fd";
  };
}
