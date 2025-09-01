{ pkgs }:


let
  xtrans = pkgs.xorg.xtrans.overrideAttrs (oldAttrs: {
    version = "1.4.0";
    src = pkgs.xorg.xtrans.src;
  });

  xcbproto = pkgs.xcb-proto.overrideAttrs (oldAttrs: {
    version = "1.14.1";
    src = pkgs.xcb-proto.src;
    extraBuildInputs = [ pkgs.xorg.libXau ];
  });

  xorgproto = pkgs.xorg.xorgproto.overrideAttrs (oldAttrs: {
    version = "1.14.1";
    src = pkgs.xorg.xorgproto.src;
  });

  libxcb = pkgs.xorg.libxcb.overrideAttrs (oldAttrs: {
    version = "1.14";
    src = pkgs.xorg.libxcb.src;
  });

  libevdev = pkgs.libevdev.overrideAttrs (oldAttrs: {
    version = "1.12.1";
    src = pkgs.libevdev.src;
  });

  libx11 = pkgs.xorg.libX11.overrideAttrs (oldAttrs: {
    version = "1.7.5";
    src = pkgs.xorg.libX11.src;
  });

  xorgServer = pkgs.xorg.xorgserver.overrideAttrs (oldAttrs: {
    version = "21.1.4";
    src = pkgs.xorg.xorgserver.src;
    patches = (oldAttrs.patches or []) ++ [
      ./patches/xorgServer/0001-Skip-checking-graphics-under-sys.patch
    ];
    nativeBuildInputs = (oldAttrs.nativeBuildInputs or []) ++ [
      pkgs.meson
      pkgs.ninja
      pkgs.pkg-config
    ];
    buildInputs = [
      libx11
      libxcb
      xtrans
      xorgproto
      pkgs.xorg.xcbutil         # provides xcb-aux
      pkgs.xorg.xcbutilimage    # provides xcb-image
      pkgs.xorg.xcbutilkeysyms  # provides xcb-keysyms
      pkgs.xorg.xcbutilrenderutil # provides xcb-renderutil
      pkgs.xorg.xcbutilwm       # provides xcb-icccm
      pkgs.xorg.utilmacros
      pkgs.xorg.libXau
      pkgs.xorg.libXdmcp
      pkgs.xorg.pixman
      pkgs.libdrm
      pkgs.mesa
      pkgs.mesa-demos
      pkgs.mesa-gl-headers
      pkgs.libGL
      pkgs.openssl
      pkgs.xorg.libxcvt
      pkgs.xorg.libXfont2
      pkgs.xorg.xdriinfo
      pkgs.xorg.libxkbfile
      pkgs.dri-pkgconfig-stub
      pkgs.xorg.libpciaccess
      pkgs.libepoxy
      pkgs.glamoroustoolkit
      pkgs.libgbm
      pkgs.xorg.xf86inputevdev
      pkgs.xorg.xf86videofbdev
      pkgs.fontconfig
      pkgs.dejavu_fonts
      pkgs.xkeyboard_config
      pkgs.xorg.fontsunmisc
      pkgs.xorg.fontutil
      pkgs.libudev-zero
      pkgs.libtirpc
    ];

    configurePhase = ''
      meson setup builddir \
        --prefix=$out \
        --libdir=lib \
        -Dxorg=true \
        -Dglamor=true \
        -Dxkb_output_dir=/var/lib/xkb \
        -Doptimization=0 \
        -Ddebug=true \
        -Dudev=false \
        -Dxkb_bin_dir=/usr/bin \
        -Dudev_kms=false
      '';
    buildPhase = ''
      meson compile -C builddir
    '';

    installPhase = ''
      meson install -C builddir
    '';
  });

  evtest = pkgs.evtest.overrideAttrs (oldAttrs: {
    version = "1.35";
    src = pkgs.evtest.src;
  });
in
{
  inherit xtrans xcbproto xorgproto libxcb libx11 libevdev xorgServer evtest;
}