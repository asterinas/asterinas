{ pkgs }:

let
  xorg = pkgs.callPackage ./xorg.nix {
    inherit pkgs;
  };

in {
  xfwm4 = pkgs.xfce.xfwm4.overrideAttrs (oldAttrs: {
    version = "4.16.1";
  });

  xfdesktop = pkgs.xfce.xfdesktop.overrideAttrs (oldAttrs: {
    version = "4.16.0";
    patches = (oldAttrs.patches or []) ++ [
      ./patches/xfdesktop4/0001-Fix-not-using-consistent-monitor-identifiers.patch
    ];
  });
}