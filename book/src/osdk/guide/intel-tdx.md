# Running an OS in Intel TDX env

The OSDK supports running your OS in an [Intel TDX](https://www.intel.com/content/www/us/en/developer/tools/trust-domain-extensions/overview.html) environment conveniently.
Intel TDX can provide a more secure environment for your OS.

## Prepare the Intel TDX Environment

Please make sure your server supports Intel TDX.

See [this guide](https://github.com/canonical/tdx/tree/noble-24.04?tab=readme-ov-file#4-setup-host-os)
or other materials to enable Intel TDX in host OS.

To verify the TDX host status, you can type:

```bash
dmesg | grep "TDX module initialized"
```

The following result is an example:

```bash
[   20.507296] tdx: TDX module initialized.
```

If you see the message "TDX module initialized",
it means the TDX module has loaded successfully.

The Intel TDX environment requires TDX-enhanced versions of QEMU, KVM, GRUB,
and other essential software for running an OS.
Therefore, it is recommended to use a Docker image to deploy the environment.

Run a TDX Docker container:

```bash
docker run -it --privileged --network=host --device=/dev/kvm asterinas/osdk:0.16.0-20250815
```

## Edit `OSDK.toml` for Intel TDX support

As Intel TDX has extra requirements or restrictions for VMs,
it demands adjusting the OSDK configurations accordingly.
This can be easily achieved with the `scheme` feature of the OSDK,
which provides a convenient way to override the default OSDK configurations
for a specific environment.

For example, you can append the following TDX-specific scheme to your `OSDK.toml` file.

```toml
[scheme."tdx"]
supported_archs = ["x86_64"]
boot.method = "grub-qcow2"
grub.mkrescue_path = "~/tdx-tools/grub"
grub.protocol = "linux"
qemu.args = """\
    -accel kvm \
    -m 8G \
    -vga none \
    -monitor pty \
    -nodefaults \
    -drive file=target/osdk/asterinas/asterinas.qcow2,if=virtio,format=qcow2 \
    -monitor telnet:127.0.0.1:9001,server,nowait \
    -bios /root/ovmf/release/OVMF.fd \
    -object tdx-guest,sept-ve-disable=on,id=tdx0 \
    -cpu host,-kvm-steal-time,pmu=off \
    -machine q35,kernel-irqchip=split,confidential-guest-support=tdx0 \
    -smp 1 \
    -nographic \
"""
```

To choose the configurations specified by the TDX scheme over the default ones,
add the `--scheme` argument to the build, run, or test command.

```bash
cargo osdk build --scheme tdx
cargo osdk run --scheme tdx
cargo osdk test --scheme tdx
```
