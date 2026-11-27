// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <linux/vt.h>
#include <linux/kd.h>
#include <sys/ioctl.h>
#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"

static int vt_fd;

#define MAX_WIDTH 64
#define MAX_HEIGHT 128
#define MAX_CHARCOUNT 512

static unsigned char buffer[MAX_WIDTH * MAX_HEIGHT * MAX_CHARCOUNT / 8];

FN_SETUP(setup)
{
	int fb_fd;

	fb_fd = open("/dev/fb0", O_RDWR);
	if (fb_fd < 0) {
		// This test does not work if the framebuffer console does not exist.
		if (errno == ENOENT || errno == ENODEV || errno == ENXIO) {
			fprintf(stderr,
				"font tests skipped: open('/dev/fb0'): %s\n",
				strerror(errno));
			exit(EXIT_SUCCESS);
		}
		fprintf(stderr, "fatal error: open('/dev/fb0'): %s\n",
			strerror(errno));
		exit(EXIT_FAILURE);
	}
	CHECK(close(fb_fd));

	vt_fd = CHECK(open("/dev/tty0", O_RDWR));
}
END_SETUP()

#define CONSOLE_FONT_OP                                       \
	((struct console_font_op){ .op = KD_FONT_OP_SET_TALL, \
				   .width = 8,                \
				   .height = 16,              \
				   .charcount = 256,          \
				   .data = buffer })

FN_TEST(kd_fontop_op)
{
	struct console_font_op op = CONSOLE_FONT_OP;

	op.op = KD_FONT_OP_SET;
	TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));
	op.op = KD_FONT_OP_SET_TALL;
	TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));

	op.op = 0xdeadbeef;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), ENOSYS);
}
END_TEST()

FN_TEST(kd_fontop_width)
{
	struct console_font_op op = CONSOLE_FONT_OP;

	for (unsigned int use_tall = 0; use_tall < 2; ++use_tall) {
		op.op = use_tall ? KD_FONT_OP_SET_TALL : KD_FONT_OP_SET;

		op.width = 1;
		TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));
		op.width = MAX_WIDTH;
		TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));

		op.width = 0;
		TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
		op.width = MAX_WIDTH + 1;
		TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
	}
}
END_TEST()

FN_TEST(kd_fontop_height)
{
	struct console_font_op op = CONSOLE_FONT_OP;

	for (unsigned int use_tall = 0; use_tall < 2; ++use_tall) {
		unsigned int max_height = use_tall ? MAX_HEIGHT : 32;
		op.op = use_tall ? KD_FONT_OP_SET_TALL : KD_FONT_OP_SET;

		op.height = 1;
		TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));
		op.height = max_height;
		TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));

		op.height = 0;
		TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
		op.height = max_height + 1;
		TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
	}
}
END_TEST()

FN_TEST(kd_fontop_charcount)
{
	struct console_font_op op = CONSOLE_FONT_OP;

	op.charcount = 256;
	TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));
	op.charcount = 512;
	TEST_SUCC(ioctl(vt_fd, KDFONTOP, &op));

	op.charcount = 0;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
	op.charcount = 255;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
	op.charcount = 257;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
	op.charcount = 511;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
	op.charcount = 513;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
}
END_TEST()

FN_TEST(kd_fontop_buffer)
{
	struct console_font_op op = CONSOLE_FONT_OP;

	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, NULL), EFAULT);
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, (void *)1), EFAULT);

	op.data = NULL;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EINVAL);
	op.data = (void *)1;
	TEST_ERRNO(ioctl(vt_fd, KDFONTOP, &op), EFAULT);
}
END_TEST()

FN_SETUP(cleanup)
{
	struct console_font_op op = { 0 };

	// Restore the default font.
	op.op = KD_FONT_OP_SET_DEFAULT;
	CHECK(ioctl(vt_fd, KDFONTOP, &op));

	CHECK(close(vt_fd));
}
END_SETUP()
