#!/usr/bin/env python3
"""Boot Debian Linux on Milk-V Megrez via serial, auto-login, and verify
network is reachable from WSL.

Usage:
    python3 boot_linux.py [/dev/ttyUSB0 [115200]]

The script:
  1. Connects to the serial console
  2. Interrupts U-Boot autoboot (if needed) or enters U-Boot commands
  3. Loads Linux kernel + DTB + initrd from SD card
  4. Boots with `booti`
  5. Waits for the login prompt, logs in as `anjie`
  6. Checks that the board has an IP address on `end1`
  7. Prints the board IP — ready for SSH/scp

Log: /tmp/serial-boot-linux.log
"""

import sys
import time
import serial
import re


def wait_for(ser, pattern, timeout=60):
    """Read until `pattern` (bytes) appears or timeout. Returns decoded text."""
    deadline = time.time() + timeout
    buf = bytearray()
    while time.time() < deadline:
        chunk = ser.read(1024)
        if chunk:
            buf.extend(chunk)
            if pattern in buf:
                return buf.decode("utf-8", errors="replace")
        else:
            time.sleep(0.05)
    return buf.decode("utf-8", errors="replace")


def send_slow(ser, text, char_delay=0.003):
    """Send text character-by-character to avoid overflowing U-Boot buffer."""
    for ch in text:
        ser.write(ch.encode())
        ser.flush()
        time.sleep(char_delay)
    ser.write(b"\n")
    ser.flush()


def send_cmd(ser, cmd, wait_pattern=b"=>", timeout=120):
    """Send a command and wait for the response."""
    print(f">>> {cmd}", flush=True)
    send_slow(ser, cmd)
    if wait_pattern is None:
        return ""
    out = wait_for(ser, wait_pattern, timeout)
    print(out[-300:] if len(out) > 300 else out, end="", flush=True)
    return out


def main():
    port = sys.argv[1] if len(sys.argv) > 1 else "/dev/ttyUSB0"
    baud = int(sys.argv[2]) if len(sys.argv) > 2 else 115200
    log_path = "/tmp/linux-boot.log"

    print(f"[*] Opening {port} @ {baud}", flush=True)

    with serial.Serial(
        port, baud,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_ONE,
        timeout=0.2,
    ) as ser, open(log_path, "w", encoding="utf-8", errors="replace") as log:

        # ── Step 1: Reach U-Boot prompt ──────────────────────────────
        print("[1] Looking for U-Boot prompt ...", flush=True)
        time.sleep(0.5)
        drain = ser.read(8192)
        decoded = drain.decode("utf-8", errors="replace") if drain else ""
        log.write(decoded)
        if decoded:
            print(decoded[-200:] if len(decoded) > 200 else decoded, end="", flush=True)

        at_prompt = False
        deadline = time.time() + 90
        buf = bytearray()
        while time.time() < deadline and not at_prompt:
            chunk = ser.read(4096)
            if chunk:
                buf.extend(chunk)
                text = chunk.decode("utf-8", errors="replace")
                log.write(text)
                if "Autoboot in" in buf.decode("utf-8", errors="replace") or \
                   "Hit any key" in buf.decode("utf-8", errors="replace"):
                    print("\n[*] Interrupting autoboot ...", flush=True)
                    ser.write(b" ")
                    ser.flush()
                    time.sleep(0.3)
                if "=>" in buf.decode("utf-8", errors="replace"):
                    at_prompt = True
            else:
                time.sleep(0.1)

        if not at_prompt:
            ser.write(b"\n")
            ser.flush()
            time.sleep(0.5)
            out = wait_for(ser, b"=>", 5)
            log.write(out)

        print("[+] At U-Boot prompt", flush=True)

        # ── Step 2: Load Linux ───────────────────────────────────────
        print("[2] Loading Linux kernel + DTB + initrd ...", flush=True)

        cmds = [
            "ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb",
            "ext4load mmc 1:1 0x80200000 /vmlinuz-6.6.87-win2030",
            "ext4load mmc 1:1 0x83000000 /initrd.img-6.6.87-win2030",
            "setenv bootargs root=/dev/mmcblk1p3 rw console=ttyS0,115200 earlycon cpu_no_boost_1_6ghz",
        ]
        for cmd in cmds:
            send_cmd(ser, cmd, b"=>", timeout=120)

        # ── Step 3: Boot ─────────────────────────────────────────────
        print("[3] Booting kernel ...", flush=True)
        send_cmd(ser, "booti 0x80200000 0x83000000:${filesize} 0xf0000000", None)

        # ── Step 4: Wait for login prompt ─────────────────────────────
        print("[4] Waiting for login prompt (up to 180s) ...", flush=True)
        out = wait_for(ser, b"login:", 180)
        log.write(out)
        print(out[-300:] if len(out) > 300 else out, end="", flush=True)

        if b"login:" not in out.encode("utf-8", errors="replace"):
            print("[!] Did not reach login prompt.", flush=True)
            return 1

        print("\n[+] Login prompt reached.", flush=True)

        # ── Step 5: Login ────────────────────────────────────────────
        print("[5] Logging in as anjie ...", flush=True)
        time.sleep(0.5)
        ser.write(b"anjie\n")
        ser.flush()
        time.sleep(0.5)
        wait_for(ser, b"Password:", 10)
        ser.write(b"passwd\n")
        ser.flush()

        # Wait for shell prompt
        out = wait_for(ser, b"$", 10)
        print("[+] Logged in.", flush=True)

        # ── Step 6: Check network ────────────────────────────────────
        print("[6] Checking network ...", flush=True)
        ser.write(b"ip addr show end1 2>/dev/null || ip addr show eth0 2>/dev/null\n")
        ser.flush()
        time.sleep(1)
        out = wait_for(ser, b"$", 10)
        log.write(out)
        # Parse IP address
        ip_match = re.search(r'inet\s+([\d.]+)', out)
        if ip_match:
            ip = ip_match.group(1)
            print(f"\n[+] Board IP: {ip}", flush=True)
        else:
            print("\n[!] Could not determine board IP from output", flush=True)

        # ── Step 7: Try ping test to verify connectivity ──────────────
        print("[7] Testing network (ping gateway) ...", flush=True)
        ser.write(b"ping -c 2 -W 2 192.168.100.1 2>&1\n")
        ser.flush()
        out = wait_for(ser, b"$", 15)
        log.write(out)
        if "bytes from" in out or "0% packet loss" in out:
            print("[+] Network working!", flush=True)
        else:
            print("[-] Ping failed — board may need manual network setup", flush=True)

        print(f"\n[*] Linux is running. Full log: {log_path}", flush=True)
        print("[*] To interact with the board:", flush=True)
        print(f"    ssh anjie@{ip if ip_match else '192.168.100.2'}", flush=True)

    return 0


if __name__ == "__main__":
    sys.exit(main())
