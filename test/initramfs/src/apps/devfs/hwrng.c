// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/fb.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../test.h"

#define HWRNG_DEVICE "/dev/hwrng"
#define HWRNG_MAJOR 10
#define HWRNG_MINOR 183

static int hwrng_fd = -1;

FN_TEST(open_hwrng)
{
	struct stat st;

	hwrng_fd = TEST_SUCC(open(HWRNG_DEVICE, O_RDONLY));

	TEST_RES(fstat(hwrng_fd, &st),
		 S_ISCHR(st.st_mode) && major(st.st_rdev) == HWRNG_MAJOR &&
			 minor(st.st_rdev) == HWRNG_MINOR);
}
END_TEST()

FN_TEST(read_hwrng)
{
	uint8_t buf1[64];
	uint8_t buf2[64];

	ssize_t ret = read(hwrng_fd, buf1, sizeof(buf1));

	if (ret < 0) {
		if (errno == ENODEV) {
			fprintf(stderr, "hwrng tests skipped: %s (%s)\n",
				HWRNG_DEVICE, strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr,
			"fatal error: read_hwrng: read('%s') failed: %s\n",
			HWRNG_DEVICE, strerror(errno));
		exit(EXIT_FAILURE);
	}

	TEST_RES(read(hwrng_fd, buf2, sizeof(buf2)), _ret == 64);

	TEST_RES(memcmp(buf1, buf2, sizeof(buf1)), _ret != 0);
}
END_TEST()

FN_SETUP(close_hwrng)
{
	CHECK(close(hwrng_fd));
}
END_SETUP()
