# SPDX-License-Identifier: MPL-2.0
#
# coreutils 9.12 for every package set built from the pinned nixpkgs.
#
# 9.11's `tee` truncates its output, which affacts the pjdfstest cases
# that pipe through it. The fix is in 9.12.

final: prev:

{
  coreutils = prev.coreutils.overrideAttrs (
    finalAttrs: previousAttrs: {
      version = "9.12";
      src = final.fetchurl {
        url = "mirror://gnu/coreutils/coreutils-${finalAttrs.version}.tar.xz";
        hash = "sha256-pIAZhVlzPps9qZnpBUOsb4iKLKpUTY1mTFofF+Uo4hA=";
      };
    }
  );
}
