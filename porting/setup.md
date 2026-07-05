# Setup Guide

How to connect to the Milk-V Megrez board, boot Linux, and get a usable network/SSH path for transferring files.

## 1. Hardware Connections

| Item | Connection |
|------|------------|
| Power | USB-C cable to the board |
| Serial | FTDI USB-UART to the debug header. In Windows it appears as a COM port; in WSL it is `/dev/ttyUSB0` after USB/IP attach. |
| Network | Ethernet cable from the board's `end1` to the host PC's Ethernet port |
| Storage | SanDisk 128 GB microSD with the pre-flashed Debian image |

### Serial Parameters

- Baud: **115200**
- Data bits: **8**
- Parity: **none**
- Stop bits: **1**
- Flow control: **none**

In WSL, attach the FTDI device with `usbipd` first (Windows side):

```powershell
usbipd bind --busid <BUSID>
usbipd attach --wsl --busid <BUSID>
```

Then in WSL:

```bash
sudo chmod 666 /dev/ttyUSB0
python3 -m serial.tools.miniterm /dev/ttyUSB0 115200
```

## 2. Booting Debian Linux from U-Boot

The board's U-Boot defaults to an autoboot command that tries to start a
`rust_shyper` image. To boot Linux, interrupt autoboot and run:

```bash
ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb
ext4load mmc 1:1 0x80200000 /vmlinuz-6.6.87-win2030
ext4load mmc 1:1 0x83000000 /initrd.img-6.6.87-win2030
setenv bootargs root=/dev/mmcblk1p3 rw console=ttyS0,115200 earlycon cpu_no_boost_1_6ghz
booti 0x80200000 0x83000000:${filesize} 0xf0000000
```

> `rust_shyper` uses `bootm`; Linux uses `booti`.

You can persist these commands to `bootcmd` if you want Linux to start
automatically.

## 3. Network Setup

The easiest path is Windows Internet Connection Sharing (ICS):

1. Share the Wi-Fi adapter to the Ethernet adapter that the board is plugged into.
2. Force ICS to use the `192.168.100.0/24` subnet by setting the registry keys:

```powershell
Set-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters" `
    -Name "ScopeAddress" -Value "192.168.100.1" -Type String -Force
Set-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters" `
    -Name "ScopeAddressBackup" -Value "192.168.100.1" -Type String -Force
```

3. Enable sharing via the HNetCfg COM object (or the Windows Settings GUI).

On the Debian side, `end1` usually comes up with DHCP and gets an address in the
`192.168.100.0/24` range. In our environment the board holds `192.168.100.2/24`.

Verify:

```bash
ip addr show end1
ping -c 3 8.8.8.8
```

## 4. SSH Access

Password authentication is broken in this Debian image (PAM/crypto issue), so use
public-key auth.

On the board:

```bash
mkdir -p /home/anjie/.ssh
chmod 700 /home/anjie/.ssh
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOWpLBjiasmXninyxyZI/MAENwbr+zb2v3fnKmowuZCh 25418@QuteWin" \
    > /home/anjie/.ssh/authorized_keys
chmod 600 /home/anjie/.ssh/authorized_keys
chown -R anjie:anjie /home/anjie/.ssh
```

From Windows:

```powershell
ssh -o StrictHostKeyChecking=no -o PasswordAuthentication=no `
    -o PreferredAuthentications=publickey anjie@192.168.100.2
```

The private key `~/.ssh/id_ed25519` is already present on this Windows machine
and in WSL under `/mnt/c/Users/25418/.ssh/`.

## 5. Board Credentials

| Account | Password | Notes |
|---------|----------|-------|
| `anjie` | `passwd` | Normal user, can `sudo` |
| `root` | `milkv` | Root access |

## 6. Transferring Files

With SSH working, use `scp` from Windows:

```powershell
scp .\aster-nix.booti anjie@192.168.100.2:/tmp/
```

Then on the board:

```bash
sudo cp /tmp/aster-nix.booti /boot/aster-nix.booti
```

Because WSL is on a different subnet (`172.23.x.x`), `scp` from WSL directly
does not reach `192.168.100.2`; use the Windows OpenSSH client from WSL:

```bash
/mnt/c/Windows/System32/OpenSSH/scp.exe /tmp/aster-nix.booti anjie@192.168.100.2:/tmp/
```
