// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ovl_sparse_copyup_test"
#define UPPER_DIR BASE_DIR "/upper"
#define WORK_DIR BASE_DIR "/work"
#define LOWER_DIR BASE_DIR "/lower"
#define MERGED_DIR BASE_DIR "/merged"

#define HOLE_SIZE (1 << 20)
#define WINDOW_SIZE (64 * 1024)
#define WINDOW_OFFSET 4096
#define TAIL "tail-bytes"
#define TAIL_LEN 10

static void create_dir(const char *path)
{
	CHECK(mkdir(path, 0755));
}

static void setup_overlay_tree(void)
{
	create_dir(BASE_DIR);
	create_dir(UPPER_DIR);
	create_dir(WORK_DIR);
	create_dir(LOWER_DIR);
	create_dir(MERGED_DIR);

	int fd = CHECK(open(LOWER_DIR "/sparse", O_WRONLY | O_CREAT, 0644));
	CHECK(lseek(fd, HOLE_SIZE, SEEK_SET));
	CHECK(write(fd, TAIL, TAIL_LEN));
	CHECK(close(fd));
}

static void cleanup_overlay_tree(void)
{
	CHECK(unlink(UPPER_DIR "/sparse"));
	CHECK(unlink(LOWER_DIR "/sparse"));

	CHECK(rmdir(MERGED_DIR));
	/* The mount leaves this staging dir behind; it survives unmount. */
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

FN_TEST(sparse_hole_and_tail_read_through)
{
	char buf[WINDOW_SIZE];
	struct stat st;

	int fd = TEST_SUCC(open(MERGED_DIR "/sparse", O_RDONLY));

	TEST_RES(fstat(fd, &st),
		 _ret == 0 && st.st_size == HOLE_SIZE + TAIL_LEN);

	TEST_RES(pread(fd, buf, sizeof(buf), WINDOW_OFFSET),
		 _ret == WINDOW_SIZE);
	long nonzero = 0;
	for (int i = 0; i < WINDOW_SIZE; i++)
		nonzero += buf[i] != 0;
	TEST_RES(nonzero, _ret == 0);

	TEST_RES(pread(fd, buf, TAIL_LEN, HOLE_SIZE), _ret == TAIL_LEN);
	TEST_RES(memcmp(buf, TAIL, TAIL_LEN), _ret == 0);

	TEST_SUCC(close(fd));
}

END_TEST()

FN_TEST(copyup_preserves_sparse_content)
{
	char buf[WINDOW_SIZE];
	struct stat st;

	int fd = TEST_SUCC(open(MERGED_DIR "/sparse", O_WRONLY));
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_RES(write(fd, "H", 1), _ret == 1);
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(MERGED_DIR "/sparse", O_RDONLY));
	TEST_RES(fstat(fd, &st),
		 _ret == 0 && st.st_size == HOLE_SIZE + TAIL_LEN);
	TEST_RES(pread(fd, buf, 1, 0), _ret == 1 && buf[0] == 'H');
	TEST_RES(pread(fd, buf, sizeof(buf), WINDOW_OFFSET),
		 _ret == WINDOW_SIZE);
	long nonzero = 0;
	for (int i = 0; i < WINDOW_SIZE; i++)
		nonzero += buf[i] != 0;
	TEST_RES(nonzero, _ret == 0);
	TEST_RES(pread(fd, buf, TAIL_LEN, HOLE_SIZE), _ret == TAIL_LEN);
	TEST_RES(memcmp(buf, TAIL, TAIL_LEN), _ret == 0);
	TEST_SUCC(close(fd));

	TEST_RES(stat(LOWER_DIR "/sparse", &st),
		 _ret == 0 && st.st_size == HOLE_SIZE + TAIL_LEN);
	fd = TEST_SUCC(open(LOWER_DIR "/sparse", O_RDONLY));
	TEST_RES(pread(fd, buf, 1, 0), _ret == 1 && buf[0] == '\0');
	TEST_RES(pread(fd, buf, TAIL_LEN, HOLE_SIZE), _ret == TAIL_LEN);
	TEST_RES(memcmp(buf, TAIL, TAIL_LEN), _ret == 0);
	TEST_SUCC(close(fd));
}

END_TEST()

FN_SETUP(cleanup)
{
	CHECK(umount(MERGED_DIR));
	cleanup_overlay_tree();
}

END_SETUP()
