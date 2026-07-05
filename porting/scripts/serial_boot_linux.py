#!/usr/bin/env python3
"""Boot Debian Linux on the Milk-V Megrez board from the U-Boot prompt.

Usage:
    python3 serial_boot_linux.py /dev/ttyUSB0 115200

The script sends the manual U-Boot commands from porting/setup.md, waits for
loads to complete, and then waits for the Debian login prompt.
"""

import sys
import time
import serial


def wait_for_prompt(ser, prompt="=>", timeout=30):
    """Read until prompt appears or timeout. Returns captured text."""
    deadline = time.time() + timeout
    buf = bytearray()
    prompt_bytes = prompt.encode()
    while time.time() < deadline:
        chunk = ser.read(1024)
        if chunk:
            buf.extend(chunk)
            if prompt_bytes in buf:
                return buf.decode("utf-8", errors="replace")
        else:
            time.sleep(0.05)
    return buf.decode("utf-8", errors="replace")


def send_slow(ser, text, char_delay=0.005):
    """Send text one character at a time to avoid overflowing U-Boot's input buffer."""
    for ch in text:
        ser.write(ch.encode())
        ser.flush()
        time.sleep(char_delay)
    ser.write(b"\n")
    ser.flush()


def send_cmd(ser, cmd, wait_for="=>", timeout=60):
    """Send a command and wait for the specified response."""
    print(f">>> {cmd}", flush=True)
    send_slow(ser, cmd)
    if wait_for is None:
        return ""
    out = wait_for_prompt(ser, wait_for, timeout)
    print(out, end="", flush=True)
    return out


def main():
    if len(sys.argv) < 2:
        port = "/dev/ttyUSB0"
        baud = 115200
    elif len(sys.argv) < 3:
        port = sys.argv[1]
        baud = 115200
    else:
        port = sys.argv[1]
        baud = int(sys.argv[2])

    log_path = "/tmp/serial-boot-linux.log"
    print(f"[*] Opening {port} @ {baud}, log -> {log_path}")

    with serial.Serial(
        port,
        baud,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_ONE,
        timeout=0.2,
    ) as ser, open(log_path, "w", encoding="utf-8", errors="replace") as log:
        # Drain stale data and make sure we are at a prompt.
        time.sleep(0.5)
        drain = ser.read(4096)
        if drain:
            text = drain.decode("utf-8", errors="replace")
            log.write(text)
            print(text, end="")

        ser.write(b"\n")
        ser.flush()
        out = wait_for_prompt(ser, "=>", 5)
        log.write(out)
        print(out, end="")

        cmds = [
            "ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb",
            "ext4load mmc 1:1 0x80200000 /vmlinuz-6.6.87-win2030",
            "ext4load mmc 1:1 0x83000000 /initrd.img-6.6.87-win2030",
            "setenv bootargs root=/dev/mmcblk1p3 rw console=ttyS0,115200 earlycon cpu_no_boost_1_6ghz",
        ]
        for cmd in cmds:
            send_cmd(ser, cmd, "=>", timeout=120)

        # The boot command starts the kernel; do not wait for => after this.
        send_cmd(ser, "booti 0x80200000 0x83000000:${filesize} 0xf0000000", None)

        print("[*] Waiting for Debian login prompt (up to 180s) ...")
        deadline = time.time() + 180
        buf = bytearray()
        login_detected = False
        while time.time() < deadline and not login_detected:
            chunk = ser.read(4096)
            if chunk:
                buf.extend(chunk)
                text = chunk.decode("utf-8", errors="replace")
                log.write(text)
                print(text, end="")
                decoded = buf.decode("utf-8", errors="replace")
                if "login:" in decoded or "rockos-eswin login" in decoded:
                    login_detected = True
            else:
                time.sleep(0.1)

        if login_detected:
            print("\n[+] Debian login prompt reached.")
        else:
            print("\n[!] Timed out waiting for login prompt.")


if __name__ == "__main__":
    main()
