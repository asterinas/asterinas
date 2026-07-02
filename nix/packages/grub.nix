# SPDX-License-Identifier: MPL-2.0
#
# GRUB from the Asterinas fork used by the Docker image. Boot images always
# target x86_64-efi, so the module tree comes from an x86_64 build (the
# caller passes grub2 from pkgsCross.gnu64). On x86_64 hosts that build also
# provides the userland tools. On other hosts the cross-built tools cannot
# run locally, so they come from a native build of the same fork instead and
# grub-mkrescue is pointed at the x86_64-efi modules explicitly.
{ stdenv, grub2, grub2-host, fetchFromGitHub, runCommand, makeWrapper }:

let
  fork = pkg:
    pkg.overrideAttrs (old: {
      version = "asterinas-2.12-0633bc8";

      src = fetchFromGitHub {
        owner = "asterinas";
        repo = "grub";
        rev = "0633bc8c08fd61b64cc19bb7ebafc871e8c4172b";
        hash = "sha256-0lefdpErGjA/4HNZ8FVXEtn2O54PD3mhqASlI2Suelg=";
      };

      # Build the fork pristine like the Docker image does. nixpkgs' 2.12
      # patch stack would otherwise apply on top of the fork and diverge from
      # the reference build, and a fork rev that merges any of those upstream
      # commits would stop building.
      patches = [ ];

      postPatch = (old.postPatch or "") + ''
        # Bake in the modules Asterinas boot images expect.
        echo "depends bli part_gpt" > grub-core/extra_deps.lst
      '';
    });

  x86_64-efi = fork grub2;
in if stdenv.hostPlatform.isx86_64 then
  x86_64-efi
else
  let tools = fork grub2-host;
  in runCommand "grub-${x86_64-efi.version}" {
    nativeBuildInputs = [ makeWrapper ];
  } ''
    mkdir -p $out/bin $out/lib/grub
    ln -s ${tools}/bin/* $out/bin/
    # OSDK invokes grub-mkrescue without --directory, so the default module
    # path baked into the native tools would be the host platform's. Point
    # it at the x86_64-efi modules the boot ISO needs. grub-mkrescue is the
    # only GRUB tool the repo invokes; the other tools keep their native
    # default module directory.
    rm $out/bin/grub-mkrescue
    makeWrapper ${tools}/bin/grub-mkrescue $out/bin/grub-mkrescue \
      --add-flags "--directory=${x86_64-efi}/lib/grub/x86_64-efi"
    ln -s ${x86_64-efi}/lib/grub/x86_64-efi $out/lib/grub/
    ln -s ${tools}/share $out/share
  ''
