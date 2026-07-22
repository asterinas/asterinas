final: prev: {
  xorg-server = prev.xorg-server.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ])
      ++ [ ./patches/xorgServer/0001-Skip-checking-graphics-under-sys.patch ];
    buildInputs = (oldAttrs.buildInputs or [ ]) ++ [ final.libudev-zero ];
    mesonFlags = (oldAttrs.mesonFlags or [ ]) ++ [
      "-Dglamor=true"
      "-Doptimization=0"
      "-Dudev=false"
      "-Dudev_kms=false"
    ];
    postInstall = (oldAttrs.postInstall or "") + ''
      mkdir -p $out/share/X11/xorg.conf.d
      cp ${
        ./patches/xorgServer/10-fbdev.conf
      } $out/share/X11/xorg.conf.d/10-fbdev.conf
    '';
  });

  xf86-video-fbdev = prev.xf86-video-fbdev.overrideAttrs (oldAttrs: {
    # The driver loads its helper modules after dlopen.
    # See https://github.com/NixOS/nixpkgs/pull/545344.
    hardeningDisable = (oldAttrs.hardeningDisable or [ ]) ++ [ "bindnow" ];
  });

  xfdesktop = prev.xfdesktop.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ]) ++ [
      ./patches/xfdesktop4/0001-Fix-not-using-consistent-monitor-identifiers.patch
    ];
  });
}
