// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <fcntl.h>
#include <termios.h>
#include <pty.h>
#include <poll.h>
#include "../../common/test.h"

int master;
int slave;
struct pollfd pfd;

#define POLL_EVENTS (POLLIN | POLLOUT | POLLRDHUP | POLLPRI | POLLRDNORM)

FN_SETUP(init)
{
	CHECK(openpty(&master, &slave, NULL, NULL, NULL));

	struct termios term;
	CHECK(tcgetattr(slave, &term));

	// Disable canonical mode and echoing.
	term.c_lflag &= ~(ICANON | ECHO);

	term.c_cc[VMIN] = 1;
	term.c_cc[VTIME] = 0;

	// Enable software flow control (IXON) and set start/stop characters.
	term.c_iflag |= IXON;
	term.c_cc[VSTOP] = '\023';
	term.c_cc[VSTART] = '\021';

	CHECK(tcsetattr(slave, TCSANOW, &term));

	pfd.fd = master;
	pfd.events = POLL_EVENTS;
}
END_SETUP()

FN_TEST(set_get_packet_mode)
{
	int packet_mode;
	TEST_RES(ioctl(master, TIOCGPKT, &packet_mode), packet_mode == 0);
	TEST_ERRNO(ioctl(slave, TIOCGPKT, &packet_mode), ENOTTY);

	packet_mode = 1;
	TEST_SUCC(ioctl(master, TIOCPKT, &packet_mode));
	TEST_ERRNO(ioctl(slave, TIOCPKT, &packet_mode), ENOTTY);

	TEST_RES(ioctl(master, TIOCGPKT, &packet_mode), packet_mode == 1);
	TEST_ERRNO(ioctl(slave, TIOCGPKT, &packet_mode), ENOTTY);
}
END_TEST()

FN_TEST(read_write)
{
	char buf[1] = { 'a' };
	TEST_SUCC(write(master, buf, sizeof(buf)));
	char read_buf[128] = { 0 };
	TEST_RES(read(slave, read_buf, sizeof(read_buf)), _ret == 1);
	TEST_SUCC(write(slave, buf, sizeof(buf)));

	TEST_RES(read(master, read_buf, 1), _ret == 1 && read_buf[0] == 0);
	TEST_RES(read(master, read_buf, 1), _ret == 1 && read_buf[0] == 0);
	TEST_RES(read(master, read_buf, sizeof(read_buf)),
		 _ret == 2 && read_buf[0] == 0 && read_buf[1] == 'a');
}
END_TEST()

FN_TEST(pkt_ioctl)
{
	// Changing the EXTPROC flag on the slave (either setting or unsetting)
	// should generate a TIOCPKT_IOCTL control packet on the master.

	struct termios term;
	TEST_SUCC(tcgetattr(slave, &term));
	term.c_lflag |= EXTPROC;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLPRI | POLLOUT | POLLRDNORM));

	char read_buf[128] = { 0 };
	TEST_RES(read(master, read_buf, sizeof(read_buf)),
		 _ret == 1 && read_buf[0] == TIOCPKT_IOCTL);

	TEST_SUCC(tcgetattr(slave, &term));
	term.c_lflag &= ~EXTPROC;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	char buf[1] = { 'a' };
	TEST_SUCC(write(slave, buf, sizeof(buf)));
	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLPRI | POLLOUT | POLLRDNORM));

	TEST_RES(read(master, read_buf, sizeof(read_buf)),
		 _ret == 1 && read_buf[0] == TIOCPKT_IOCTL);
	TEST_RES(read(master, read_buf, sizeof(read_buf)),
		 _ret == 2 && read_buf[0] == 0 && read_buf[1] == 'a');
}
END_TEST()

FN_TEST(pkt_nostop)
{
	// Disabling IXON on the slave should generate a TIOCPKT_NOSTOP packet.

	struct termios term;
	TEST_SUCC(tcgetattr(slave, &term));
	term.c_iflag &= ~IXON;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLPRI | POLLOUT | POLLRDNORM));

	char buf[1] = { 'a' };
	TEST_SUCC(write(slave, buf, sizeof(buf)));
	char read_buf[128] = { 0 };
	TEST_RES(read(master, read_buf, sizeof(read_buf)),
		 _ret == 1 && read_buf[0] == TIOCPKT_NOSTOP);

	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDNORM));
	TEST_RES(read(master, read_buf, sizeof(read_buf)),
		 _ret == 2 && read_buf[1] == 'a');
}
END_TEST()

FN_TEST(pkt_dostop)
{
	// Enabling IXON on the slave should generate a TIOCPKT_DOSTOP packet.

	struct termios term;
	TEST_SUCC(tcgetattr(slave, &term));
	term.c_iflag |= IXON;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLPRI | POLLOUT | POLLRDNORM));

	char read_buf[128] = { 0 };
	TEST_RES(read(master, read_buf, sizeof(read_buf)),
		 _ret == 1 && read_buf[0] == TIOCPKT_DOSTOP);
}
END_TEST()

FN_TEST(close_and_reopen_slave)
{
	// Closing the pty slave will not reset the packet status.

	struct termios term;
	TEST_SUCC(tcgetattr(slave, &term));
	term.c_lflag |= EXTPROC;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	TEST_RES(poll(&pfd, 1, -1),
		 pfd.revents == (POLLIN | POLLPRI | POLLOUT | POLLRDNORM));

	TEST_SUCC(close(slave));

	int packet_mode;
	TEST_RES(ioctl(master, TIOCGPKT, &packet_mode), packet_mode == 1);

	slave = TEST_SUCC(ioctl(master, TIOCGPTPEER, NULL));
	TEST_RES(ioctl(master, TIOCGPKT, &packet_mode), packet_mode == 1);

	char buf[128];
	TEST_RES(read(master, buf, sizeof(buf)),
		 _ret == 1 && buf[0] == TIOCPKT_IOCTL);
}
END_TEST()

FN_TEST(no_data_read)
{
	int flags = TEST_SUCC(fcntl(master, F_GETFL, 0));
	TEST_SUCC(fcntl(master, F_SETFL, flags | O_NONBLOCK));

	char buf[1];
	TEST_ERRNO(read(master, buf, 1), EAGAIN);
	TEST_SUCC(close(slave));
	TEST_ERRNO(read(master, buf, 1), EIO);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(master));
}
END_SETUP()
