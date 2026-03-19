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
#include <sys/wait.h>
#include <unistd.h>

#include "../common/test.h"

#define FB_DEVICE "/dev/fb0"
#define PAGE_SIZE 4096
#define CMAP_LEN 4

static int fb_fd = -1;
static size_t fb_smem_len;

FN_SETUP(open_framebuffer)
{
	struct fb_fix_screeninfo fix_info;
	struct fb_var_screeninfo var_info;

	fb_fd = open(FB_DEVICE, O_RDWR);
	if (fb_fd < 0) {
		if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
			fprintf(stderr, "framebuffer tests skipped: %s (%s)\n",
				FB_DEVICE, strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr,
			"fatal error: setup_open_framebuffer: open('%s') failed: %s\n",
			FB_DEVICE, strerror(errno));
		exit(EXIT_FAILURE);
	}

	CHECK_WITH(ioctl(fb_fd, FBIOGET_FSCREENINFO, &fix_info),
		   _ret == 0 && fix_info.smem_len != 0);
	CHECK_WITH(ioctl(fb_fd, FBIOGET_VSCREENINFO, &var_info),
		   _ret == 0 && var_info.xres != 0);

	fb_smem_len = fix_info.smem_len;
}
END_SETUP()

FN_TEST(color_map)
{
	uint16_t red_expected[CMAP_LEN];
	uint16_t green_expected[CMAP_LEN];
	uint16_t blue_expected[CMAP_LEN];

	for (size_t i = 0; i < CMAP_LEN; ++i) {
		red_expected[i] = 0x0100 + i;
		green_expected[i] = 0x0200 + i;
		blue_expected[i] = 0x0300 + i;
	}

	uint16_t red[CMAP_LEN];
	uint16_t green[CMAP_LEN];
	uint16_t blue[CMAP_LEN];

	memcpy(red, red_expected, sizeof(red));
	memcpy(green, green_expected, sizeof(green));
	memcpy(blue, blue_expected, sizeof(blue));

	struct fb_cmap set_cmap = {
		.start = 0,
		.len = CMAP_LEN,
		.red = red,
		.green = green,
		.blue = blue,
		.transp = NULL,
	};

	TEST_SUCC(ioctl(fb_fd, FBIOPUTCMAP, &set_cmap));

	memset(red, 0, sizeof(red));
	memset(green, 0, sizeof(green));
	memset(blue, 0, sizeof(blue));

	struct fb_cmap get_cmap = {
		.start = 0,
		.len = CMAP_LEN,
		.red = red,
		.green = green,
		.blue = blue,
		.transp = NULL,
	};

	TEST_SUCC(ioctl(fb_fd, FBIOGETCMAP, &get_cmap));
	TEST_RES(memcmp(red, red_expected, sizeof(red)), _ret == 0);
	TEST_RES(memcmp(green, green_expected, sizeof(green)), _ret == 0);
	TEST_RES(memcmp(blue, blue_expected, sizeof(blue)), _ret == 0);

	// Test invalid color map: use a start index that's definitely out of range
	// Most devices have 256 entries or less, so start=0x10000 should always fail
	struct fb_cmap invalid_cmap = {
		.start = 0x10000,
		.len = 1,
		.red = red,
		.green = green,
		.blue = blue,
		.transp = NULL,
	};

	TEST_ERRNO(ioctl(fb_fd, FBIOPUTCMAP, &invalid_cmap), EINVAL);
}
END_TEST()

FN_TEST(write_enospc)
{
	const unsigned char test_pattern = 0xff;

	// First, read the last byte to preserve it
	unsigned char original_byte = 0;
	TEST_RES(lseek(fb_fd, (off_t)(fb_smem_len - 1), SEEK_SET),
		 _ret == (off_t)(fb_smem_len - 1));
	TEST_RES(read(fb_fd, &original_byte, 1), _ret == 1);

	// Now seek to the end and try to write beyond the end of the framebuffer
	TEST_RES(lseek(fb_fd, (off_t)fb_smem_len, SEEK_SET),
		 _ret == (off_t)fb_smem_len);
	TEST_ERRNO(write(fb_fd, &test_pattern, sizeof(test_pattern)), ENOSPC);

	// Finally, verify that the last byte is unchanged
	TEST_RES(lseek(fb_fd, (off_t)(fb_smem_len - 1), SEEK_SET),
		 _ret == (off_t)(fb_smem_len - 1));
	unsigned char check_byte = 0;
	TEST_RES(read(fb_fd, &check_byte, 1), _ret == 1);
	TEST_RES(memcmp(&check_byte, &original_byte, sizeof(check_byte)),
		 _ret == 0);
}
END_TEST()

FN_TEST(mmap_mremap_and_fork)
{
	static uint8_t pattern[PAGE_SIZE];
	static uint8_t fork_pattern[PAGE_SIZE];

	size_t map_len = fb_smem_len;
	if (map_len > sizeof(pattern)) {
		map_len = sizeof(pattern);
	}

	for (size_t i = 0; i < map_len; ++i) {
		pattern[i] = (uint8_t)(i & 0xff);
		fork_pattern[i] = (uint8_t)(0xaa ^ (i & 0xff));
	}

	uint8_t *mapped = TEST_SUCC((uint8_t *)mmap(
		NULL, map_len, PROT_READ | PROT_WRITE, MAP_SHARED, fb_fd, 0));
	memcpy(mapped, pattern, map_len);
	TEST_RES(memcmp(mapped, pattern, map_len), _ret == 0);

	uint8_t *remapped = TEST_SUCC(
		(uint8_t *)mremap(mapped, map_len, map_len, MREMAP_MAYMOVE));
	if (remapped != mapped) {
		mapped = remapped;
	}

	TEST_RES(memcmp(mapped, pattern, map_len), _ret == 0);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		// Child process: avoid TEST_* so the parent sees the status via waitpid
		CHECK_WITH(memcmp(mapped, pattern, map_len), _ret == 0);
		memcpy(mapped, fork_pattern, map_len);
		_exit(EXIT_SUCCESS);
	}

	// Parent process
	int status = 0;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
	TEST_RES(memcmp(mapped, fork_pattern, map_len), _ret == 0);

	TEST_RES(munmap(mapped, map_len), _ret == 0);
}
END_TEST()

FN_SETUP(close_framebuffer)
{
	CHECK(close(fb_fd));
}
END_SETUP()
