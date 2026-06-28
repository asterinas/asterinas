self: super:

{
  xorg-server = super.xorg-server.overrideAttrs (oldAttrs: {
    version = "21.1.23";
    src = oldAttrs.src;
    patches = (oldAttrs.patches or [ ])
      ++ [ ./patches/xorgServer/0001-Skip-checking-graphics-under-sys.patch ];
    nativeBuildInputs = (oldAttrs.nativeBuildInputs or [ ])
      ++ [ self.meson self.ninja self.pkg-config ];
    buildInputs = (oldAttrs.buildInputs or [ ]) ++ [
      self.dri-pkgconfig-stub
      self.libudev-zero
      self.font-util
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
        -Dxkb_bin_dir=${self.xkbcomp}/bin \
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

  xorg = super.xorg // { xorgserver = self.xorg-server; };

  xfwm4 = super.xfwm4;

  xfdesktop = super.xfdesktop.overrideAttrs (oldAttrs: {
    patches = (oldAttrs.patches or [ ]) ++ [
      ./patches/xfdesktop4/0001-Fix-not-using-consistent-monitor-identifiers.patch
    ];
  });

  xfce = super.xfce // {
    xfwm4 = self.xfwm4;
    xfdesktop = self.xfdesktop;
  };
}
