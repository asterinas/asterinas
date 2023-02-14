### How to compile this busybox
We don't include the source code of busybox here since the source code is really large. The busybox can be compiled with following commands.

After download the source code of busybox 1.35.0 and unzip, then cd to the directory of busybox
1. `make defconfig`. We set all config as default.
2. Set static link option in .config: `CONFIG_STATIC=y`. We need a static-linked busybox binary since we does not support dynamic linking now.
3. Set standalone shell option in .config: `CONFIG_FEATURE_SH_STANDALONE=y`. The standalone ash will try to call busybox applets instead of search binaries in host system. e.g., when running ls, standalone ash will invoke `busybox ls`.