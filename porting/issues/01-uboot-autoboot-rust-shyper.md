# Issue 01: U-Boot autoboot tries `rust_shyper` instead of Linux

## Symptom

On power-on, U-Boot executes the default `bootcmd` and attempts to start an SD
card image named `rust_shyper`. It does not boot Debian Linux automatically.

## Cause

The default U-Boot environment on this board is configured to boot a
hypervisor-like image (`rust_shyper`) via `bootm`, not the Debian Linux kernel.

## Fix

Interrupt autoboot by pressing any key during the countdown, then manually load
Linux:

```bash
ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb
ext4load mmc 1:1 0x80200000 /vmlinuz-6.6.87-win2030
ext4load mmc 1:1 0x83000000 /initrd.img-6.6.87-win2030
setenv bootargs root=/dev/mmcblk1p3 rw console=ttyS0,115200 earlycon cpu_no_boost_1_6ghz
booti 0x80200000 0x83000000:${filesize} 0xf0000000
```

To make Linux the default, persist these commands to `bootcmd`:

```bash
setenv bootcmd 'ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb; ext4load mmc 1:1 0x80200000 /vmlinuz-6.6.87-win2030; ext4load mmc 1:1 0x83000000 /initrd.img-6.6.87-win2030; setenv bootargs root=/dev/mmcblk1p3 rw console=ttyS0,115200 earlycon cpu_no_boost_1_6ghz; booti 0x80200000 0x83000000:${filesize} 0xf0000000'
saveenv
```

> Use `booti` for Linux, not `bootm`.

## Verification

After running the manual commands or saving `bootcmd`, Debian Linux reaches the
`rockos-eswin login:` prompt.
