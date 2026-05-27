// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/vfs.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/sparse_test"
#define PAGE_SIZE 4096

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

FN_SETUP(prepare_base_dir)
{
	ensure_dir(BASE_DIR);
}
END_SETUP()

FN_TEST(pwrite_pread_hole_zeros)
{
	const char *path = BASE_DIR "/pwrite_pread";
	const char *data = "ABCD";
	char buf[4] = { 0 };
	char zero_buf[4] = { 0 };

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR | O_TRUNC, 0644));
	TEST_RES(pwrite(fd, data, 4, 100), _ret == 4);

	TEST_RES(pread(fd, buf, 4, 100), _ret == 4);
	TEST_RES(memcmp(buf, data, 4), _ret == 0);

	memset(buf, 0xff, sizeof(buf));
	TEST_RES(pread(fd, buf, 4, 0), _ret == 4);
	TEST_RES(memcmp(buf, zero_buf, 4), _ret == 0);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(sparse_hole_zero)
{
	const char *path = BASE_DIR "/sparse_hole";
	char buf[PAGE_SIZE];

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR | O_TRUNC, 0644));
	TEST_RES(pwrite(fd, "data", 4, PAGE_SIZE * 10), _ret == 4);

	memset(buf, 0xff, sizeof(buf));
	TEST_RES(pread(fd, buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);

	int all_zero = 1;
	for (int i = 0; i < PAGE_SIZE; i++) {
		if (buf[i] != 0) {
			all_zero = 0;
			break;
		}
	}
	TEST_RES(all_zero, _ret == 1);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(sparse_direct_hole_zero)
{
	const char *path = BASE_DIR "/sparse_direct";
	char *buf;

	buf = aligned_alloc(PAGE_SIZE, PAGE_SIZE);
	CHECK_WITH(buf == NULL ? -1 : 0, _ret == 0);

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR | O_TRUNC, 0644));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE * 4));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(path, O_RDONLY | O_DIRECT));
	memset(buf, 0xff, PAGE_SIZE);
	TEST_RES(pread(fd, buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);

	int all_zero = 1;
	for (int i = 0; i < PAGE_SIZE; i++) {
		if (buf[i] != 0) {
			all_zero = 0;
			break;
		}
	}
	TEST_RES(all_zero, _ret == 1);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
	free(buf);
}
END_TEST()

FN_TEST(truncate_shrink_extend_zeros)
{
	const char *path = BASE_DIR "/shrink_extend";
	char buf[PAGE_SIZE];

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR | O_TRUNC, 0644));

	memset(buf, 'A', PAGE_SIZE);
	TEST_RES(write(fd, buf, PAGE_SIZE), _ret == PAGE_SIZE);

	TEST_SUCC(ftruncate(fd, 100));
	TEST_SUCC(ftruncate(fd, PAGE_SIZE));

	memset(buf, 0xff, PAGE_SIZE);
	TEST_RES(pread(fd, buf, PAGE_SIZE, 0), _ret == PAGE_SIZE);

	int tail_zero = 1;
	for (int i = 100; i < PAGE_SIZE; i++) {
		if (buf[i] != 0) {
			tail_zero = 0;
			break;
		}
	}
	TEST_RES(tail_zero, _ret == 1);

	for (int i = 0; i < 100; i++) {
		if (buf[i] != 'A') {
			tail_zero = 0;
			break;
		}
	}
	TEST_RES(tail_zero, _ret == 1);

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(statfs_bavail_reserved)
{
	struct statfs st;

	TEST_SUCC(statfs("/ext2", &st));
	TEST_RES(statfs("/ext2", &st), st.f_bavail <= st.f_bfree);
}
END_TEST()
