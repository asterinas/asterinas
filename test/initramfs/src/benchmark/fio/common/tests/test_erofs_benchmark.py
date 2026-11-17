#!/usr/bin/env python3

# SPDX-License-Identifier: MPL-2.0

"""Check EROFS profiles and condition records without building or booting a VM."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


BENCHMARK = Path(__file__).resolve().parents[3]
REPOSITORY = BENCHMARK.parents[3]
FILE_BYTES = 211951616


class ErofsBenchmarkTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="erofs-benchmark-test.")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.benchmark = self.root / "test/initramfs/src/benchmark"
        shutil.copytree(BENCHMARK, self.benchmark)
        self.build = self.root / "test/initramfs/build"
        self.work = self.build / "erofs-fio/work"
        (self.work / "root").mkdir(parents=True)
        with (self.work / "root/silesia.tar").open("wb") as dataset:
            dataset.truncate(FILE_BYTES)
        dataset_hash = hashlib.sha256((self.work / "root/silesia.tar").read_bytes()).hexdigest()
        (self.work / "dataset.txt").write_text(
            f"source_url=synthetic-fixture\nfile_bytes={FILE_BYTES}\n"
            f"file_sha256={dataset_hash}\narchive_sha256={'2' * 64}\n"
        )
        for profile in ("plain", "lz4hc_c4096"):
            image = self.work / f"{profile}-fixture.img"
            image.write_bytes(b"synthetic image")
            (self.work / f"{profile}.img").symlink_to(image.name)
            Path(str(image) + ".txt").write_text(
                f"profile={profile}\nsource_file=silesia.tar\n"
                f"mkfs_options=synthetic-fixture\ntool hash\n{dataset_hash}  {self.work}/root/silesia.tar\nmkfs.erofs fixture\n"
            )
            Path(str(image) + ".sha256").write_text(hashlib.sha256(image.read_bytes()).hexdigest() + f"  {image}\n")
        (self.build / "initramfs.cpio.gz").write_bytes(b"synthetic initramfs")
        guest_fio = self.build / "initramfs/benchmark/bin/fio"
        self.write_executable(guest_fio, "#!/bin/sh\necho fio-3.41\n")
        kernel = self.root / "linux"
        kernel.mkdir()
        (kernel / "bzImage").write_bytes(b"synthetic Linux kernel")
        bundle = self.root / "target/osdk/asterinas/asterinas-osdk-bin"
        bundle.parent.mkdir(parents=True)
        bundle.write_bytes(b"synthetic Asterinas kernel")
        (bundle.parent / "bundle.toml").write_text('[initramfs]\npath = "initramfs-image"\n')
        (bundle.parent / "initramfs-image").write_bytes(b"synthetic initramfs")
        body = {
            "fio version": "fio-3.41",
            "jobs": [{
                "error": 0,
                "job options": {
                    "filename": "/erofs/silesia.tar",
                    "rw": "randread",
                    "ioengine": "sync",
                    "size": str(FILE_BYTES),
                    "io_size": str(FILE_BYTES),
                    "bs": "4096",
                    "direct": "0",
                    "numjobs": "1",
                    "iodepth": "1",
                    "loops": "1",
                    "time_based": "0",
                    "ramp_time": "0",
                    "invalidate": "0",
                    "fsync_on_close": "0",
                    "allow_file_create": "0",
                    "randrepeat": "1",
                    "randseed": "20261110",
                    "random_generator": "tausworthe",
                    "norandommap": "0",
                },
                "read": {
                    "bw_bytes": 100000000,
                    "io_bytes": FILE_BYTES,
                    "total_ios": FILE_BYTES // 4096,
                    "short_ios": 0,
                    "drop_ios": 0,
                    "runtime": 100,
                },
                "write": {"io_bytes": 0},
            }],
        }
        (self.root / "fio.json").write_text(json.dumps(body) + "\n")
        self.write_executable(self.root / "bin/nix-build", f"#!/bin/sh\necho {kernel}\n")
        self.write_executable(self.root / "bin/git", "#!/bin/sh\necho synthetic-head\n")
        self.write_executable(self.root / "bin/make", """#!/bin/sh
if [ "$1" = run_kernel ]; then
    if [ "${TEST_TIMEOUT:-0}" = 1 ]; then
        [ "${TEST_GUEST_STARTED:-0}" != 1 ] || echo 'GUEST_UNAME: synthetic Asterinas'
        sleep 30
    fi
    if [ "${TEST_INITRAMFS_DRIFT:-0}" = 1 ]; then
        printf 'changed initramfs' > "$TEST_ROOT/test/initramfs/build/initramfs.cpio.gz"
        cp "$TEST_ROOT/test/initramfs/build/initramfs.cpio.gz" "$TEST_ROOT/target/osdk/asterinas/initramfs-image"
    fi
    exec "$TEST_ROOT/bin/emit-guest"
fi
""")
        self.write_executable(self.root / "bin/qemu-system-x86_64", """#!/bin/sh
if [ "$1" = --version ]; then echo synthetic-qemu; exit 0; fi
exec "$TEST_ROOT/bin/emit-guest"
""")
        self.write_executable(self.root / "bin/emit-guest", """#!/bin/sh
echo 'GUEST_UNAME: synthetic Linux 6.18.39'
echo "EROFS_PROFILE=$EROFS_FIO_PROFILE"
echo FIO_RESULT_BEGIN
cat "$TEST_ROOT/fio.json"
echo FIO_RESULT_END
echo BENCHMARK_EXIT_STATUS=0
if [ "${TEST_ESCAPE_END:-0}" = 1 ]; then printf '\\033['; fi
""")
        self.environment = dict(os.environ, TEST_ROOT=str(self.root),
                                PATH=str(self.root / "bin") + ":" + os.environ["PATH"])
        for key in ("LINUX_KERNEL", "EROFS_FIO_IMAGE", "EROFS_FIO_PROFILE", "EROFS_FIO_WORK_DIR",
                    "EROFS_FIO_FILE_BYTES", "BENCH_SYSTEM_ORDER", "BENCH_TIMEOUT", "BENCHMARK_ROOT", "INITRAMFS_SKIP_GZIP"):
            self.environment.pop(key, None)

    @staticmethod
    def write_executable(path, contents):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)
        path.chmod(0o755)

    def run_pair(self, profile, **extra_environment):
        output = self.root / "result"
        environment = dict(self.environment, BENCH_OUTPUT_DIR=str(output), **extra_environment)
        command = ["bash", str(self.benchmark / "bench_linux_and_aster.sh"),
                   f"fio/erofs_randread_bw_{profile}"]
        result = subprocess.run(command, cwd=self.root, env=environment, capture_output=True, text=True)
        return result, output

    def test_plain_and_compressed_results_match_ext2_location(self):
        for profile in ("plain", "lz4hc_c4096"):
            with self.subTest(profile=profile):
                result, output = self.run_pair(profile)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                results = json.loads((self.root / f"result_fio-erofs_randread_bw_{profile}.json").read_text())
                self.assertEqual(len(results), 2)
                if profile == "plain":
                    self.assertNotIn("LZ4HC", json.dumps(results))
                self.assertFalse(output.exists())

    def test_rebuilt_aster_initramfs_drift_rejects_pair(self):
        result, output = self.run_pair("plain", TEST_INITRAMFS_DRIFT="1", BENCH_SYSTEM_ORDER="linux,asterinas")
        self.assertNotEqual(result.returncode, 0)
        conditions = (output / "conditions.md").read_text()
        self.assertIn("LINUX_INITRAMFS_SHA256=" + hashlib.sha256(b"synthetic initramfs").hexdigest(), conditions)
        self.assertIn("ASTER_INITRAMFS_SHA256=" + hashlib.sha256(b"changed initramfs").hexdigest(), conditions)
        self.assertIn("INITRAMFS_MATCH=0", conditions)
        self.assertTrue((output / "linux_fio.json").exists())
        self.assertTrue((output / "aster_fio.json").exists())
        self.assertFalse((output / "result_fio-erofs_randread_bw_plain.json").exists())

    def test_uncompressed_initramfs_is_rejected_before_boot(self):
        result, output = self.run_pair("plain", INITRAMFS_SKIP_GZIP="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gzip initramfs", result.stderr)
        self.assertIn("整体退出码：`1`", (output / "conditions.md").read_text())
        self.assertFalse((output / "aster_output.txt").exists())
        self.assertFalse((output / "linux_output.txt").exists())

    def test_profile_mismatch_retains_failed_conditions(self):
        yaml = self.benchmark / "fio/erofs_randread_bw_plain/bench_result.yaml"
        yaml.write_text(yaml.read_text().replace("image_profile: plain", "image_profile: lz4hc_c4096"))
        result, output = self.run_pair("plain")
        self.assertNotEqual(result.returncode, 0)
        conditions = (output / "conditions.md").read_text()
        self.assertIn("整体退出码：`1`", conditions)
        self.assertIn("未启动", conditions)

    def test_partial_console_escape_does_not_hide_host_exit_status(self):
        result, output = self.run_pair("plain", TEST_ESCAPE_END="1", TEST_INITRAMFS_DRIFT="1",
                                       BENCH_SYSTEM_ORDER="linux,asterinas")
        self.assertNotEqual(result.returncode, 0)
        conditions = (output / "conditions.md").read_text()
        self.assertEqual(conditions.count("HOST_EXIT_STATUS=0"), 2)
        for system in ("aster", "linux"):
            self.assertIn(b"\x1b[\nHOST_EXIT_STATUS=0\n", (output / f"{system}_output.txt").read_bytes())

    def test_timeout_retains_command_and_exit_status(self):
        result, output = self.run_pair("plain", TEST_TIMEOUT="1", BENCH_TIMEOUT="0.1")
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        conditions = (output / "conditions.md").read_text()
        self.assertIn("整体退出码：`124`", conditions)
        self.assertIn("HOST_EXIT_STATUS=124", conditions)
        self.assertIn("ASTER_KERNEL_IDENTITY=unconfirmed", conditions)
        self.assertNotIn("ASTER_KERNEL_SHA256=", conditions)
        self.assertIn("make run_kernel", conditions)
        self.assertIn("未启动", conditions)
        self.assertFalse((output / "result_fio-erofs_randread_bw_plain.json").exists())

    def test_linux_json_and_started_aster_kernel_survive_later_timeout(self):
        result, output = self.run_pair("plain", TEST_TIMEOUT="1", TEST_GUEST_STARTED="1",
                                       BENCH_TIMEOUT="0.1", BENCH_SYSTEM_ORDER="linux,asterinas")
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertTrue((output / "linux_fio.json").exists())
        self.assertFalse((output / "aster_fio.json").exists())
        self.assertFalse((output / "result_fio-erofs_randread_bw_plain.json").exists())
        conditions = (output / "conditions.md").read_text()
        self.assertIn("ASTER_KERNEL_SHA256=" + hashlib.sha256(b"synthetic Asterinas kernel").hexdigest(), conditions)
        self.assertIn("整体退出码：`124`", conditions)

    def test_wrapper_preserves_default_argv(self):
        tool = self.root / "fio-tool"
        self.write_executable(tool, """#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
Path(os.environ['TEST_ROOT'], 'argv.json').write_text(json.dumps(sys.argv[1:]))
print('{"jobs": []}')
""")
        wrapper = self.root / "run_fio.sh"
        wrapper.write_text((BENCHMARK / "fio/common/run_fio.sh").read_text().replace("/benchmark/bin/fio", str(tool)))
        options = ["--rw=read", "--filename=/ext2/fio-test"]
        expected = options + ["--output-format=json", "--eta=never"]
        result = subprocess.run(["sh", str(wrapper), *options], env=self.environment, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads((self.root / "argv.json").read_text()), expected)
        self.assertEqual(result.stdout, 'FIO_RESULT_BEGIN\n{"jobs": []}\nFIO_RESULT_END\n')

    def test_direct_make_defaults_and_profile_mismatch(self):
        makefile = self.root / "profile.mk"
        makefile.write_text(f"include {REPOSITORY}/Makefile\nprint_profile:\n\t@printf '%s|%s\\n' \"$(EROFS_FIO_PROFILE)\" \"$(EROFS_FIO_IMAGE)\"\n")
        for profile in ("plain", "lz4hc_c4096"):
            result = subprocess.run(["/usr/bin/make", "--no-print-directory", "-f", str(makefile),
                                     "print_profile", f"BENCHMARK=fio/erofs_randread_bw_{profile}"],
                                    cwd=REPOSITORY, env=self.environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(result.stdout.strip().endswith(f"/{profile}.img"), result.stdout)
            self.assertTrue(result.stdout.startswith(profile + "|"))
        result = subprocess.run(["/usr/bin/make", "-f", str(makefile), "print_profile",
                                 "BENCHMARK=fio/erofs_randread_bw_plain", "EROFS_FIO_PROFILE=lz4hc_c4096"],
                                cwd=REPOSITORY, env=self.environment, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
