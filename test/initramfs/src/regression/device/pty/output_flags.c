// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <unistd.h>
#include <fcntl.h>
#include <termios.h>
#include <pty.h>
#include <poll.h>
#include "../../common/test.h"

static int master;
static int slave;

// Echo path: OPOST|ONLCR (default) — \n is converted to \r\n.
FN_TEST(echo_onlcr_enabled)
{
	struct termios term;
	TEST_SUCC(openpty(&master, &slave, NULL, NULL, NULL));

	// Use default termios: OPOST|ONLCR is on, ECHO is on, ICANON is on.
	TEST_RES(tcgetattr(slave, &term),
		 (term.c_oflag & OPOST) && (term.c_oflag & ONLCR));

	// Write "A\r" to master (simulates typing 'A' then Enter).
	// ICRNL converts \r to \n; ONLCR echoes \n as \r\n.
	TEST_SUCC(write(master, "A\r", 2));

	// Read the echo from master.
	char buf[16] = { 0 };
	struct pollfd pfd = { .fd = master, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, -1), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(master, buf, sizeof(buf)), _ret == 3 && buf[0] == 'A' &&
							 buf[1] == '\r' &&
							 buf[2] == '\n');

	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));
}
END_TEST()

// Echo path: ONLCR off — \n is kept as \n.
FN_TEST(echo_onlcr_disabled)
{
	struct termios term;
	TEST_SUCC(openpty(&master, &slave, NULL, NULL, NULL));

	TEST_SUCC(tcgetattr(slave, &term));
	term.c_oflag &= ~ONLCR;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	// Write just "\r" (Enter). ICRNL converts to \n, echoed as \n.
	TEST_SUCC(write(master, "\r", 1));

	char buf[16] = { 0 };
	struct pollfd pfd = { .fd = master, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, -1), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(master, buf, sizeof(buf)), _ret == 1 && buf[0] == '\n');

	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));
}
END_TEST()

// Echo path: OPOST|OCRNL, ICRNL off — \r is converted to \n.
FN_TEST(echo_ocrnl_enabled)
{
	struct termios term;
	TEST_SUCC(openpty(&master, &slave, NULL, NULL, NULL));

	// Disable ICRNL so \r is not converted to \n on input.
	// Enable OCRNL so echoed \r is mapped to \n.
	TEST_SUCC(tcgetattr(slave, &term));
	term.c_iflag &= ~ICRNL;
	term.c_oflag |= OCRNL;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	// Write just "\r". ICRNL is off so \r stays as \r, OCRNL echoes it as \n.
	TEST_SUCC(write(master, "\r", 1));

	char buf[16] = { 0 };
	struct pollfd pfd = { .fd = master, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, -1), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(master, buf, sizeof(buf)), _ret == 1 && buf[0] == '\n');

	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));
}
END_TEST()

// Write path: OPOST|ONLCR (default) — \n is converted to \r\n.
FN_TEST(write_onlcr_enabled)
{
	struct termios term;
	TEST_SUCC(openpty(&master, &slave, NULL, NULL, NULL));

	TEST_SUCC(tcgetattr(slave, &term));
	// Turn off echo so only the write-path output is visible.
	term.c_lflag &= ~ECHO;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	// Write to slave; output flags should be applied to what master reads.
	TEST_SUCC(write(slave, "A\n", 2));

	char buf[16] = { 0 };
	struct pollfd pfd = { .fd = master, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, -1), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(master, buf, sizeof(buf)), _ret == 3 && buf[0] == 'A' &&
							 buf[1] == '\r' &&
							 buf[2] == '\n');

	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));
}
END_TEST()

// Write path: ONLCR off — \n is kept as \n.
FN_TEST(write_onlcr_disabled)
{
	struct termios term;
	TEST_SUCC(openpty(&master, &slave, NULL, NULL, NULL));

	TEST_SUCC(tcgetattr(slave, &term));
	term.c_lflag &= ~ECHO;
	term.c_oflag &= ~ONLCR;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	TEST_SUCC(write(slave, "A\n", 2));

	char buf[16] = { 0 };
	struct pollfd pfd = { .fd = master, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, -1), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(master, buf, sizeof(buf)),
		 _ret == 2 && buf[0] == 'A' && buf[1] == '\n');

	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));
}
END_TEST()

// Write path: OPOST|OCRNL — \r is converted to \n.
FN_TEST(write_ocrnl_enabled)
{
	struct termios term;
	TEST_SUCC(openpty(&master, &slave, NULL, NULL, NULL));

	TEST_SUCC(tcgetattr(slave, &term));
	term.c_lflag &= ~ECHO;
	term.c_oflag |= OCRNL;
	TEST_SUCC(tcsetattr(slave, TCSANOW, &term));

	// Write "\r" to slave; OCRNL maps it to \n on output.
	TEST_SUCC(write(slave, "\r", 1));

	char buf[16] = { 0 };
	struct pollfd pfd = { .fd = master, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, -1), _ret == 1 && (pfd.revents & POLLIN));
	TEST_RES(read(master, buf, sizeof(buf)), _ret == 1 && buf[0] == '\n');

	TEST_SUCC(close(master));
	TEST_SUCC(close(slave));
}
END_TEST()
