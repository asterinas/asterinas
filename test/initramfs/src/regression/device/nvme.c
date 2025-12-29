// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "../common/test.h"

#define DEVICE_PATH "/dev/nvme0n1"
#define MOUNT_POINT "/nvme"
#define TEST_FILE "/nvme/nvme_test"
#define BUFFER_SIZE 65536
#define ALIGNMENT 4096

#ifndef O_DIRECT
#define O_DIRECT 040000
#endif

static char *nvme_write_buf;
static char *nvme_read_buf;

FN_SETUP(mount_nvme_ext2)
{
	if (mkdir(MOUNT_POINT, 0755) != 0 && errno != EEXIST) {
		fprintf(stderr,
			"fatal error: setup_mount_nvme_ext2: mkdir('%s') failed: %s\n",
			MOUNT_POINT, strerror(errno));
		exit(EXIT_FAILURE);
	}

	if (mount(DEVICE_PATH, MOUNT_POINT, "ext2", 0, "") != 0) {
		if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
			fprintf(stderr,
				"nvme tests skipped: mount('%s') failed: %s\n",
				DEVICE_PATH, strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr,
			"fatal error: setup_mount_nvme_ext2: mount('%s') failed: %s\n",
			DEVICE_PATH, strerror(errno));
		exit(EXIT_FAILURE);
	}
}
END_SETUP()

FN_SETUP(alloc_nvme_buffers)
{
	CHECK_WITH(posix_memalign((void **)&nvme_write_buf, ALIGNMENT,
				  BUFFER_SIZE),
		   _ret == 0);
	CHECK_WITH(posix_memalign((void **)&nvme_read_buf, ALIGNMENT,
				  BUFFER_SIZE),
		   _ret == 0);
}
END_SETUP()

FN_TEST(rw_direct_verify)
{
	srand((unsigned int)time(NULL));
	for (size_t i = 0; i < BUFFER_SIZE; i++) {
		nvme_write_buf[i] = (char)(rand() % 256);
	}

	int fd = TEST_SUCC(
		open(TEST_FILE, O_CREAT | O_TRUNC | O_RDWR | O_DIRECT, 0644));
	TEST_RES(write(fd, nvme_write_buf, BUFFER_SIZE), _ret == BUFFER_SIZE);
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	TEST_RES(read(fd, nvme_read_buf, BUFFER_SIZE), _ret == BUFFER_SIZE);
	TEST_RES(memcmp(nvme_write_buf, nvme_read_buf, BUFFER_SIZE), _ret == 0);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(teardown_nvme)
{
	free(nvme_read_buf);
	nvme_read_buf = NULL;
	free(nvme_write_buf);
	nvme_write_buf = NULL;
	CHECK(umount(MOUNT_POINT));
}
END_SETUP()
