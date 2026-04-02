// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <termios.h>
#include <pty.h>
#include <poll.h>
#include <sys/wait.h>
#include "../../common/test.h"

#define DEV_PTMX "/dev/ptmx"

int master;
int slave;
char slave_name[128];

FN_SETUP(open_ptmx)
{
	master = CHECK(open(DEV_PTMX, O_RDWR));
	int slave_index;
	CHECK(ioctl(master, TIOCGPTN, &slave_index));
	memset(slave_name, 0, sizeof(slave_name));
	sprintf(slave_name, "/dev/pts/%d", slave_index);
}
END_SETUP()

FN_TEST(read_write_before_open_slave)
{
	// Set master nonblocking mode
	int flags = TEST_SUCC(fcntl(master, F_GETFL, 0));
	TEST_SUCC(fcntl(master, F_SETFL, flags | O_NONBLOCK));

	char buf[1] = { 'a' };
	TEST_ERRNO(read(master, buf, sizeof(buf)), EAGAIN);
	TEST_SUCC(write(master, buf, 1));
}
END_TEST()

FN_TEST(clear_lock_and_open_slave)
{
	TEST_ERRNO(open(slave_name, O_RDWR), EIO);
	TEST_ERRNO(ioctl(master, TIOCGPTPEER, NULL), EIO);

	// Unlock pty lock
	int lock;
	TEST_RES(ioctl(master, TIOCGPTLCK, &lock), lock == 1);
	lock = 0;
	TEST_SUCC(ioctl(master, TIOCSPTLCK, &lock));
	TEST_RES(ioctl(master, TIOCGPTLCK, &lock), lock == 0);

	slave = TEST_SUCC(open(slave_name, O_RDWR));
	int tmp_sfd = TEST_SUCC(ioctl(master, TIOCGPTPEER, NULL));
	TEST_SUCC(close(tmp_sfd));
}
END_TEST()

FN_TEST(read_write)
{
	// Set master blocking mode
	int flags = TEST_SUCC(fcntl(master, F_GETFL, 0));
	TEST_SUCC(fcntl(master, F_SETFL, flags & (~O_NONBLOCK)));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		char buf[1] = { 0 };
		CHECK_WITH(read(master, buf, sizeof(buf)),
			   _ret == 1 && buf[0] == 'a');
		exit(EXIT_SUCCESS);
	}

	// Set slave raw mode
	struct termios term;
	TEST_SUCC(tcgetattr(slave, &term));
	term.c_lflag &= ~(ICANON | ECHO);
	term.c_cc[VMIN] = 1;
	term.c_cc[VTIME] = 0;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	// Read the byte master has written.
	char buf[1] = { 0 };
	TEST_RES(read(slave, buf, sizeof(buf)), _ret == 1 && buf[0] == 'a');

	TEST_SUCC(write(slave, buf, sizeof(buf)));

	TEST_SUCC(wait(NULL));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(master));
	CHECK(close(slave));
}
END_SETUP()
