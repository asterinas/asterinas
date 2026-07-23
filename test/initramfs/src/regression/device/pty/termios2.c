// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <poll.h>
#include <pty.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

#include "../../common/test.h"

#define NCCS2 19

struct termios2 {
	tcflag_t c_iflag;
	tcflag_t c_oflag;
	tcflag_t c_cflag;
	tcflag_t c_lflag;
	cc_t c_line;
	cc_t c_cc[NCCS2];
	speed_t c_ispeed;
	speed_t c_ospeed;
};

#define TCGETS2 _IOR('T', 0x2A, struct termios2)
#define TCSETS2 _IOW('T', 0x2B, struct termios2)
#define TCSETSW2 _IOW('T', 0x2C, struct termios2)
#define TCSETSF2 _IOW('T', 0x2D, struct termios2)

static int master;
static int slave;

FN_SETUP(open_pty)
{
	CHECK(openpty(&master, &slave, NULL, NULL, NULL));
}
END_SETUP()

FN_TEST(set_termios_via_master)
{
	struct termios2 termios;
	struct termios2 observed;

	TEST_SUCC(ioctl(master, TCGETS2, &termios));
	termios.c_lflag &= ~ECHO;
	termios.c_cc[VMIN] = 2;
	termios.c_cc[VTIME] = 3;
	TEST_SUCC(ioctl(master, TCSETS2, &termios));
	TEST_RES(ioctl(slave, TCGETS2, &observed),
		 observed.c_lflag == termios.c_lflag &&
			 observed.c_cc[VMIN] == termios.c_cc[VMIN] &&
			 observed.c_cc[VTIME] == termios.c_cc[VTIME]);
}
END_TEST()

FN_TEST(set_termios_wait_via_master)
{
	struct termios2 termios;
	struct termios2 observed;

	TEST_SUCC(ioctl(master, TCGETS2, &termios));
	termios.c_lflag |= ECHO;
	termios.c_cc[VMIN] = 4;
	termios.c_cc[VTIME] = 5;
	TEST_SUCC(ioctl(master, TCSETSW2, &termios));
	TEST_RES(ioctl(slave, TCGETS2, &observed),
		 observed.c_lflag == termios.c_lflag &&
			 observed.c_cc[VMIN] == termios.c_cc[VMIN] &&
			 observed.c_cc[VTIME] == termios.c_cc[VTIME]);
}
END_TEST()

FN_TEST(flush_pending_input)
{
	struct termios2 termios;
	struct pollfd pfd = {
		.fd = slave,
		.events = POLLIN,
	};
	int bytes;

	TEST_SUCC(write(master, "discard\n", 8));
	TEST_RES(poll(&pfd, 1, -1), pfd.revents == POLLIN);
	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 8);
	TEST_SUCC(ioctl(master, TCGETS2, &termios));
	TEST_SUCC(ioctl(master, TCSETSF2, &termios));
	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 0);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(master));
	CHECK(close(slave));
}
END_SETUP()
