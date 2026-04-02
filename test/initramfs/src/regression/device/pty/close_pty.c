// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <unistd.h>
#include <fcntl.h>
#include <termios.h>
#include <pty.h>
#include <poll.h>
#include "../../common/test.h"

int master;
int slave;
char slave_name[256];

#define POLL_EVENTS (POLLIN | POLLOUT | POLLRDHUP | POLLPRI | POLLRDNORM)

void open_and_set_slave_raw_mode()
{
	memset(slave_name, 0, sizeof(slave_name));
	CHECK(openpty(&master, &slave, slave_name, NULL, NULL));
	struct termios term;
	CHECK(tcgetattr(slave, &term));
	term.c_lflag &= ~(ICANON | ECHO);
	term.c_cc[VMIN] = 1;
	term.c_cc[VTIME] = 0;
	CHECK(tcsetattr(slave, TCSANOW, &term));
}

FN_TEST(dup_then_closed)
{
	open_and_set_slave_raw_mode();

	char buf[1] = { 'a' };

	int dupped_master = TEST_SUCC(dup(master));
	int dupped_slave = TEST_SUCC(dup(slave));
	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));

	TEST_SUCC(write(dupped_master, buf, sizeof(buf)));
	TEST_SUCC(write(dupped_slave, buf, sizeof(buf)));
	struct pollfd in_pfd = { .fd = dupped_master, .events = POLLIN };
	TEST_RES(poll(&in_pfd, 1, -1), in_pfd.revents == POLLIN);
	in_pfd.fd = dupped_slave;
	TEST_RES(poll(&in_pfd, 1, -1), in_pfd.revents == POLLIN);

	TEST_RES(read(dupped_master, buf, sizeof(buf)),
		 _ret == 1 && buf[0] == 'a');
	TEST_RES(read(dupped_slave, buf, sizeof(buf)),
		 _ret == 1 && buf[0] == 'a');

	TEST_SUCC(close(dupped_master));
	TEST_SUCC(close(dupped_slave));
}
END_TEST()

FN_TEST(close_slave)
{
	open_and_set_slave_raw_mode();

	char buf[1] = { 'b' };
	int bytes;
	struct pollfd pfd = { .fd = master, .events = POLL_EVENTS };

	TEST_SUCC(write(slave, buf, sizeof(buf)));
	struct pollfd in_pfd = { .fd = master, .events = POLLIN };
	TEST_RES(poll(&in_pfd, 1, -1), in_pfd.revents == POLLIN);

	TEST_SUCC(close(slave));

	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLOUT | POLLHUP | POLLRDNORM));
	TEST_RES(ioctl(master, FIONREAD, &bytes), bytes == 1);
	TEST_RES(read(master, buf, sizeof(buf)), _ret == 1 && buf[0] == 'b');

	TEST_RES(poll(&pfd, 1, -1), pfd.revents == (POLLOUT | POLLHUP));
	TEST_RES(ioctl(master, FIONREAD, &bytes), bytes == 0);
	TEST_ERRNO(read(master, buf, sizeof(buf)), EIO);
	TEST_RES(read(master, buf, 0), _ret == 0);
	TEST_RES(write(master, buf, sizeof(buf)), _ret == 1);

	TEST_SUCC(close(master));
}
END_TEST()

FN_TEST(unlink_slave)
{
	open_and_set_slave_raw_mode();

	TEST_ERRNO(unlink(slave_name), EPERM);
	TEST_SUCC(close(slave));
	TEST_ERRNO(unlink(slave_name), EPERM);

	TEST_SUCC(close(master));

	TEST_ERRNO(unlink(slave_name), ENOENT);
}
END_TEST()

FN_TEST(close_master)
{
	open_and_set_slave_raw_mode();

	char buf[1] = { 'c' };
	int bytes;
	struct pollfd pfd = { .fd = slave, .events = POLL_EVENTS };

	TEST_SUCC(write(master, buf, sizeof(buf)));
	struct pollfd in_pfd = { .fd = slave, .events = POLLIN };
	TEST_RES(poll(&in_pfd, 1, -1), in_pfd.revents == POLLIN);

	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDNORM));
	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 1);

	TEST_SUCC(close(master));
	TEST_ERRNO(unlink(slave_name), ENOENT);

	TEST_RES(poll(&pfd, 1, -1), pfd.revents == (POLLIN | POLLOUT | POLLERR |
						    POLLHUP | POLLRDNORM));
	TEST_ERRNO(ioctl(slave, FIONREAD, &bytes), EIO);
	TEST_RES(read(slave, buf, sizeof(buf)), _ret == 0);
	TEST_ERRNO(write(slave, buf, sizeof(buf)), EIO);
	TEST_ERRNO(write(slave, buf, 0), EIO);

	TEST_SUCC(close(slave));
}
END_TEST()

FN_TEST(reopen_slave_after_close)
{
	open_and_set_slave_raw_mode();

	char buf[1] = { 'd' };
	int bytes;
	struct pollfd pfd = { .fd = master, .events = POLL_EVENTS };
	struct pollfd in_pfd = { .events = POLLIN };

	TEST_SUCC(close(slave));

	TEST_RES(write(master, buf, sizeof(buf)), _ret == 1);

	slave = TEST_SUCC(open(slave_name, O_RDWR));
	int slave2 = TEST_SUCC(open(slave_name, O_RDWR));

	in_pfd.fd = slave2;
	TEST_RES(poll(&in_pfd, 1, -1), in_pfd.revents == POLLIN);
	TEST_RES(ioctl(slave2, FIONREAD, &bytes), bytes == 1);
	TEST_RES(read(slave, buf, sizeof(buf)), _ret == 1 && buf[0] == 'd');
	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 0);

	TEST_SUCC(write(slave2, buf, sizeof(buf)));
	in_pfd.fd = master;
	TEST_RES(poll(&in_pfd, 1, -1), in_pfd.revents == POLLIN);
	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDNORM));
	TEST_RES(ioctl(master, FIONREAD, &bytes), bytes == 1);
	TEST_RES(read(master, buf, sizeof(buf)), _ret == 1 && buf[0] == 'd');

	TEST_SUCC(write(master, buf, sizeof(buf)));
	in_pfd.fd = slave;
	TEST_RES(poll(&in_pfd, 1, -1), in_pfd.revents == POLLIN);

	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 1);
	TEST_RES(ioctl(slave2, FIONREAD, &bytes), bytes == 1);
	TEST_RES(read(slave2, buf, sizeof(buf)), _ret == 1 && buf[0] == 'd');
	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 0);
	TEST_RES(ioctl(slave2, FIONREAD, &bytes), bytes == 0);

	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));
	TEST_SUCC(close(slave2));
}
END_TEST()
