// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ovl_readdir_test"
#define UPPER_DIR BASE_DIR "/upper"
#define WORK_DIR BASE_DIR "/work"
#define LOWER_DIR BASE_DIR "/lower"
#define MERGED_DIR BASE_DIR "/merged"

#define NUM_UPPER_EXTRA 3
#define NUM_LOWER_EXTRA 5
#define ONE_LONG_DIRENT_BUF_SIZE (sizeof(struct linux_dirent64) + NAME_MAX + 1)

struct linux_dirent64 {
	uint64_t d_ino;
	int64_t d_off;
	unsigned short d_reclen;
	unsigned char d_type;
	char d_name[];
};

struct readdir_result {
	int normal_file;
	int normal_extra;
	int another_file;
	int another_extra;
	int deleted;
	int whiteout;
	int total_entries;
};

static void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

static void write_file(const char *path)
{
	int fd = CHECK(open(path, O_WRONLY | O_CREAT, 0644));
	CHECK(write(fd, "data", 4));
	CHECK(close(fd));
}

static void setup_overlay_tree(void)
{
	create_dir(BASE_DIR);
	create_dir(UPPER_DIR);
	create_dir(WORK_DIR);
	create_dir(LOWER_DIR);
	create_dir(MERGED_DIR);

	write_file(UPPER_DIR "/normal_file");
	/*
	 * A Linux-format whiteout: a char device 0:0 named exactly like the
	 * deleted file. The merged view must hide both the whiteout itself and
	 * the same-named lower entry.
	 */
	CHECK(mknod(UPPER_DIR "/deleted", S_IFCHR | 0644, makedev(0, 0)));
	write_file(UPPER_DIR "/normal_extra_0");
	write_file(UPPER_DIR "/normal_extra_1");
	write_file(UPPER_DIR "/normal_extra_2");

	write_file(LOWER_DIR "/deleted");
	write_file(LOWER_DIR "/another_file");
	write_file(LOWER_DIR "/another_extra_0");
	write_file(LOWER_DIR "/another_extra_1");
	write_file(LOWER_DIR "/another_extra_2");
	write_file(LOWER_DIR "/another_extra_3");
	write_file(LOWER_DIR "/another_extra_4");
}

static void cleanup_overlay_tree(void)
{
	CHECK(unlink(UPPER_DIR "/normal_file"));
	CHECK(unlink(UPPER_DIR "/deleted"));
	CHECK(unlink(UPPER_DIR "/normal_extra_0"));
	CHECK(unlink(UPPER_DIR "/normal_extra_1"));
	CHECK(unlink(UPPER_DIR "/normal_extra_2"));

	CHECK(unlink(LOWER_DIR "/deleted"));
	CHECK(unlink(LOWER_DIR "/another_file"));
	CHECK(unlink(LOWER_DIR "/another_extra_0"));
	CHECK(unlink(LOWER_DIR "/another_extra_1"));
	CHECK(unlink(LOWER_DIR "/another_extra_2"));
	CHECK(unlink(LOWER_DIR "/another_extra_3"));
	CHECK(unlink(LOWER_DIR "/another_extra_4"));

	CHECK(rmdir(MERGED_DIR));
	/*
	 * The overlay mount creates a `work` staging directory inside the
	 * workdir (Linux `ovl_workdir_create`), which survives unmount; remove
	 * it before rmdir-ing the workdir itself.
	 */
	CHECK(rmdir(WORK_DIR "/work"));
	CHECK(rmdir(WORK_DIR));
	CHECK(rmdir(UPPER_DIR));
	CHECK(rmdir(LOWER_DIR));
	CHECK(rmdir(BASE_DIR));
}

static void mount_overlay(void)
{
	char options[256];
	snprintf(options, sizeof(options), "lowerdir=%s,upperdir=%s,workdir=%s",
		 LOWER_DIR, UPPER_DIR, WORK_DIR);

	CHECK(mount("overlay", MERGED_DIR, "overlay", 0, options));
}

FN_SETUP(init)
{
	setup_overlay_tree();
	mount_overlay();
}

END_SETUP()

FN_TEST(readdir_small_buffer)
{
	int fd = TEST_SUCC(open(MERGED_DIR, O_RDONLY | O_DIRECTORY));

	/*
	 * Use a buffer that can hold only one maximal-name dirent, so each
	 * getdents64() call may stop after a single merged entry. The merged
	 * directory view should still remain complete: the upper whiteout for
	 * `deleted` (a char device 0:0) must stay hidden, the lower `deleted`
	 * it hides must not appear, and every other entry must appear exactly
	 * once.
	 */
	char buf[ONE_LONG_DIRENT_BUF_SIZE];
	struct readdir_result result = { 0 };

	for (;;) {
		int nread =
			TEST_RES(syscall(SYS_getdents64, fd, buf, sizeof(buf)),
				 _ret >= 0);
		if (nread == 0)
			break;

		for (int pos = 0; pos < nread;) {
			struct linux_dirent64 *d =
				(struct linux_dirent64 *)(buf + pos);
			const char *name = d->d_name;

			if (strcmp(name, "normal_file") == 0) {
				result.normal_file++;
			} else if (strncmp(name, "normal_extra_", 13) == 0) {
				result.normal_extra++;
			} else if (strcmp(name, "another_file") == 0) {
				result.another_file++;
			} else if (strncmp(name, "another_extra_", 14) == 0) {
				result.another_extra++;
			} else if (strcmp(name, "deleted") == 0) {
				result.deleted++;
			} else if (strncmp(name, ".wh.", 4) == 0) {
				result.whiteout++;
			}

			result.total_entries++;
			pos += d->d_reclen;
		}
	}

	TEST_SUCC(close(fd));
	TEST_RES(result.deleted, _ret == 0);
	TEST_RES(result.whiteout, _ret == 0);
	TEST_RES(result.normal_file, _ret == 1);
	TEST_RES(result.normal_extra, _ret == NUM_UPPER_EXTRA);
	TEST_RES(result.another_file, _ret == 1);
	TEST_RES(result.another_extra, _ret == NUM_LOWER_EXTRA);
	TEST_RES(result.total_entries,
		 _ret == 2 + 1 + NUM_UPPER_EXTRA + 1 + NUM_LOWER_EXTRA);
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
}

END_SETUP()
