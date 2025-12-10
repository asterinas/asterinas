self: super:

{
  xorg = super.xorg // {
    xorgserver = super.xorg.xorgserver.overrideAttrs (oldAttrs: {
      version = "21.1.4";
      src = oldAttrs.src;
      patches = (oldAttrs.patches or [ ]) ++ [
        ./patches/xorgServer/0001-Skip-checking-graphics-under-sys.patch
        ./patches/xorgServer/0002-hardcode-tty1-usage-due-to-Asterinas-limitations.patch
      ];
      nativeBuildInputs = (oldAttrs.nativeBuildInputs or [ ])
        ++ [ self.meson self.ninja self.pkg-config ];
      buildInputs = (oldAttrs.buildInputs or [ ]) ++ [
        self.dri-pkgconfig-stub
        self.libudev-zero
        self.xorg.fontutil
        self.libtirpc
      ];
      configurePhase = ''
        meson setup builddir \
          --prefix=$out \
          --libdir=lib \
          -Dxorg=true \
          -Dglamor=true \
          -Dxkb_output_dir=$out/share/X11/xkb \
          -Doptimization=0 \
          -Dudev=false \
          -Dxkb_bin_dir=${self.xorg.xkbcomp}/bin \
          -Dudev_kms=false
      '';
      buildPhase = ''
        meson compile -C builddir
      '';
      installPhase = ''
        meson install -C builddir
        mkdir -p $out/share/X11/xorg.conf.d
        cp ${
          ./patches/xorgServer/10-fbdev.conf
        } $out/share/X11/xorg.conf.d/10-fbdev.conf
      '';
    });
  };

  xfce = super.xfce // {
    xfwm4 = super.xfce.xfwm4.overrideAttrs (oldAttrs: { version = "4.16.1"; });

    xfdesktop =
      super.xfce.xfdesktop.overrideAttrs (oldAttrs: { version = "4.16.0"; });
  };
}

