# SPDX-License-Identifier: MPL-2.0

"""Check storage and runtime policy through the regular securityfs interface."""

import errno
import os
from pathlib import Path
import subprocess
import sys


ROOT = Path("/sys/kernel/security/apparmor")
APP_PROFILES = Path(__file__).with_name("profiles")


def compile_policy(version, source):
    data = subprocess.run(
        [
            "apparmor_parser",
            "--config-file=/etc/apparmor/parser.conf",
            "-m", f"policy {{versions {{v{version} {{yes}}}}}}",
            "-j0", "-K", "-S",
        ],
        input=source.encode(),
        stdout=subprocess.PIPE,
        check=True,
        timeout=30,
    ).stdout
    assert int.from_bytes(data[12:16], "little") & 0x3ff == version
    return data


def write_securityfs(name, data):
    fd = os.open(ROOT / name, os.O_WRONLY | os.O_CLOEXEC)
    try:
        assert os.write(fd, data) == len(data)
    finally:
        os.close(fd)


def reject(blob, expected, before):
    try:
        write_securityfs(".load", blob)
    except OSError as exc:
        assert exc.errno == expected, (expected, exc)
    else:
        raise AssertionError("invalid or unsupported policy loaded")
    assert (ROOT / "profiles").read_bytes() == before


def check_profile(name, mode, blob, expected_label=None):
    profiles = ROOT / "profiles"
    data_dir = ROOT / "policy" / "profiles"
    before = profiles.read_bytes()
    previous_dirs = set(data_dir.iterdir())
    write_securityfs(".load", blob)
    try:
        listing = profiles.read_text().splitlines()
        assert f"{name} ({mode})" in listing, listing
        directories = set(data_dir.iterdir()) - previous_dirs
        assert len(directories) == 1, directories
        assert (directories.pop() / "raw_data").read_bytes() == blob
        reject(blob, errno.EEXIST, profiles.read_bytes())

        if expected_label is not None:
            check_exec(name, expected_label)
    finally:
        write_securityfs(".remove", os.fsencode(name))
    assert profiles.read_bytes() == before
    assert set(data_dir.iterdir()) == previous_dirs


def check_exec(executable, expected_label):
    child = subprocess.run(
        [executable, "-c", """
import os
with open('/proc/self/attr/current') as attr:
    print(attr.read().strip())
os.chroot('/')
"""],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        timeout=30,
    )
    assert child.stdout.splitlines() == [expected_label], child.stdout


def check_generated_profile(executable, rule, mode, expected_label):
    flags = "" if mode == "enforce" else f" flags=({mode})"
    blob = compile_policy(5, f'profile "{executable}"{flags} {{ {rule} }}\n')
    check_profile(executable, mode, blob, expected_label)


def main():
    before = (ROOT / "profiles").read_bytes()
    legacy = compile_policy(5, "profile legacy_storage_check { }\n")
    modern = compile_policy(7, "profile modern_storage_check { }\n")

    reject(modern, errno.EPROTONOSUPPORT, before)
    reject(legacy[:-1], errno.EPROTO, before)
    reject(legacy + modern, errno.EOPNOTSUPP, before)

    executable = str(Path(sys.executable).resolve())
    check_generated_profile(executable, "", "enforce", f"{executable} (enforce)")
    check_generated_profile(executable, "capability sys_chroot,", "enforce", "unconfined")
    print("APPARMOR_M3_BINDING_CONTRAST_OK")
    check_generated_profile(executable, "deny capability sys_chroot,", "enforce", "unconfined")
    for mode in ("enforce", "complain", "kill", "unconfined"):
        check_generated_profile(executable, "audit capability sys_chroot,", mode, "unconfined")
    print("APPARMOR_PROFILE_MODES_OK")

    for name in ("curl", "tinyproxy", "proftpd"):
        blob = (APP_PROFILES / f"{name}.bin").read_bytes()
        assert int.from_bytes(blob[12:16], "little") == 5, name
        check_profile(name, "enforce", blob)
        print(f"APPARMOR_M3_ABI_V5_PROFILE_OK {name} {len(blob)} bytes")
    print("APPARMOR_POLICY_STORAGE_OK")


if __name__ == "__main__":
    main()
