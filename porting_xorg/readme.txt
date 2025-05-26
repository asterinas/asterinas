1. apt-get source xorg-server
2. apply patches in xorg-server folder
3. build xorg-server and install
For example: 
```bash
meson setup builddir --prefix=/usr-xorg \
    -Dxkb_output_dir=/var/lib/xkb \
    -Doptimization=0 \
    -Ddebug=true
ninja -C builddir install
```

