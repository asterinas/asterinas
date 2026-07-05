#!/usr/bin/env python3
"""Boot the Asterinas booti image on the Milk-V Megrez board from U-Boot.

Usage:
    python3 serial_boot_asterinas.py /dev/ttyUSB0 115200

The script waits at the U-Boot prompt, loads /aster-nix.booti and the Megrez
DTB from the SD card boot partition, then runs `booti`. Output is saved to
/tmp/serial-boot-asterinas.log.
"""

import sys
import time
import serial


def send_slow(ser, text, char_delay=0.005):
    for ch in text:
        ser.write(ch.encode())
        ser.flush()
        time.sleep(char_delay)
    ser.write(b"\n")
    ser.flush()


def wait_for_prompt(ser, prompt, timeout=60):
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


def send_cmd(ser, cmd, wait_for="=>", timeout=120):
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

    log_path = "/tmp/serial-boot-asterinas.log"
    print(f"[*] Opening {port} @ {baud}, log -> {log_path}", flush=True)

    with serial.Serial(
        port,
        baud,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_ONE,
        timeout=0.2,
    ) as ser, open(log_path, "w", encoding="utf-8", errors="replace") as log:
        # If the board is already at U-Boot prompt, just continue. Otherwise
        # try to interrupt autoboot by sending a key when the countdown appears.
        print("[*] Waiting for U-Boot prompt/autoboot countdown ...", flush=True)
        deadline = time.time() + 90
        buf = bytearray()
        at_prompt = False
        while time.time() < deadline and not at_prompt:
            chunk = ser.read(1024)
            if chunk:
                buf.extend(chunk)
                text = chunk.decode("utf-8", errors="replace")
                log.write(text)
                print(text, end="", flush=True)
                decoded = buf.decode("utf-8", errors="replace")
                if "=>" in decoded:
                    at_prompt = True
                elif "Hit any key to stop autoboot" in decoded:
                    # Interrupt autoboot
                    ser.write(b" ")
                    ser.flush()
            else:
                time.sleep(0.05)

        if not at_prompt:
            # Send a newline in case we are at a prompt but missed it
            ser.write(b"\n")
            ser.flush()
            time.sleep(0.5)
            out = wait_for_prompt(ser, "=>", 5)
            log.write(out)
            print(out, end="", flush=True)

        send_cmd(ser, "ext4load mmc 1:1 0xf0000000 /dtbs/linux-image-6.6.87-win2030/eswin/eic7700-milkv-megrez.dtb", "=>")
        send_cmd(ser, "ext4load mmc 1:1 0x80200000 /aster-nix.booti", "=>")
        send_cmd(ser, "booti 0x80200000 - 0xf0000000", None)

        print("[*] Capturing Asterinas boot output (up to 300s) ...", flush=True)
        deadline = time.time() + 300
        buf = bytearray()
        markers = ["A", "E", "F", "G", "D", "H", "B"]
        found = ""
        reset_seen = False
        while time.time() < deadline:
            chunk = ser.read(4096)
            if chunk:
                buf.extend(chunk)
                text = chunk.decode("utf-8", errors="replace")
                log.write(text)
                print(text, end="", flush=True)
                decoded = buf.decode("utf-8", errors="replace")
                for m in markers:
                    if m in decoded and m not in found:
                        found += m
                if "U-Boot" in decoded and not reset_seen:
                    reset_seen = True
                    print("\n[*] Board appears to have reset (U-Boot banner seen).", flush=True)
            else:
                time.sleep(0.1)

        print(f"\n[*] Capture finished. Marker prefix seen so far: {found!r}", flush=True)
        if reset_seen:
            print("[*] SBI timer reset was triggered.", flush=True)


if __name__ == "__main__":
    main()
