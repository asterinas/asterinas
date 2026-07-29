{ lib, rustPlatform, fetchCrate, }:
rustPlatform.buildRustPackage rec {
  pname = "vhost-device-vsock";
  version = "0.3.0";

  src = fetchCrate {
    inherit pname version;
    hash = "sha256-JY18fZdVYAu8zlFGhvlNv3NkcSkZaIpWhH6DjNkDQRU=";
  };

  cargoLock.lockFileContents = builtins.readFile "${src}/Cargo.lock";
  doCheck = false;

  meta = {
    description = "Virtio-vsock device using the vhost-user protocol";
    homepage = "https://github.com/rust-vmm/vhost-device";
    license = with lib.licenses; [ asl20 bsd3 ];
    mainProgram = "vhost-device-vsock";
  };
}
