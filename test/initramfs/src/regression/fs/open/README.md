# Open permission regression tests

`open_append_only` needs an ext2 file with the append-only inode flag. The test
skips when `/ext2/open_append_only_target` is absent. To prepare this fixture,
stop QEMU, ensure the image is not mounted or in use, and create a new empty
file in the image with `debugfs`. For the default regression ext2 image:

```sh
image=test/initramfs/build/ext2.img
debugfs -R 'stat /open_append_only_target' "$image"
```

If the file does not exist, create it using an empty regular file and set its
flags. Do not run these commands if the name already exists:

```sh
fixture=$(mktemp)
debugfs -w -R "write $fixture /open_append_only_target" "$image"
debugfs -w -R 'set_inode_field /open_append_only_target flags 0x20' "$image"
debugfs -R 'stat /open_append_only_target' "$image"
```

The temporary file can be removed after preparing the fixture. Do not modify
the image while QEMU or another process is using it.
