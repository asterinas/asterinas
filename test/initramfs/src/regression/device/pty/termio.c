// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <asm/termbits.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

// glibc 2.42 removed struct termio from <sys/ioctl.h>.
#ifndef NCC
#define NCC 8
struct termio {
	unsigned short c_iflag;
	unsigned short c_oflag;
	unsigned short c_cflag;
	unsigned short c_lflag;
	unsigned char c_line;
	unsigned char c_cc[NCC];
};
#endif

static int master;
static int slave;
static struct termios2 initial;
static const unsigned long setters[] = { TCSETA, TCSETAW, TCSETAF };

FN_SETUP(open_pty)
{
	master = CHECK(posix_openpt(O_RDWR | O_NOCTTY));
	CHECK(grantpt(master));
	CHECK(unlockpt(master));
	slave = CHECK(open(ptsname(master), O_RDWR | O_NOCTTY | O_NONBLOCK));
	CHECK(ioctl(slave, TCGETS2, &initial));
}
END_SETUP()

FN_TEST(get_legacy_attributes)
{
	int fds[] = { master, slave };
	struct termio result;

	for (size_t i = 0; i < 2; ++i) {
		memset(&result, 0xa5, sizeof(result));
		TEST_RES(ioctl(fds[i], TCGETA, &result),
			 _ret == 0 &&
				 result.c_iflag == (uint16_t)initial.c_iflag &&
				 result.c_oflag == (uint16_t)initial.c_oflag &&
				 result.c_cflag == (uint16_t)initial.c_cflag &&
				 result.c_lflag == (uint16_t)initial.c_lflag &&
				 result.c_line == initial.c_line &&
				 !memcmp(result.c_cc, initial.c_cc, NCC) &&
				 ((unsigned char *)&result)[17] == 0);
	}
}
END_TEST()

FN_TEST(set_legacy_attributes_preserves_modern_state)
{
	struct termios2 before = initial;
	struct termios2 after = { 0 };
	struct termio legacy = { 0 };

	// Include unknown flag bits, extended control characters and BOTHER speeds.
	before.c_iflag |= 0x80000000;
	before.c_oflag |= 0x40000000;
	before.c_lflag |= EXTPROC;
	before.c_cflag &= ~(CBAUD | CIBAUD);
	before.c_cflag |= CRTSCTS | BOTHER | (BOTHER << IBSHIFT);
	before.c_ispeed = 123456;
	before.c_ospeed = 654321;
	for (size_t k = NCC; k < NCCS; ++k)
		before.c_cc[k] = 0x40 + k;
	TEST_SUCC(ioctl(slave, TCSETS2, &before));
	TEST_SUCC(ioctl(master, TCGETA, &legacy));

	legacy.c_iflag ^= ICRNL | 0x8000;
	legacy.c_oflag ^= ONLCR | TAB3;
	legacy.c_lflag ^= ECHO;
	legacy.c_line = 0x42;
	for (size_t k = 0; k < NCC; ++k)
		legacy.c_cc[k] = 0x20 + k;
	TEST_SUCC(ioctl(master, TCSETA, &legacy));
	TEST_RES(ioctl(slave, TCGETS2, &after),
		 after.c_iflag == ((before.c_iflag & 0xffff0000) |
				   legacy.c_iflag) &&
			 after.c_oflag == ((before.c_oflag & 0xffff0000) |
					   legacy.c_oflag) &&
			 after.c_cflag == before.c_cflag &&
			 after.c_lflag == ((before.c_lflag & 0xffff0000) |
					   legacy.c_lflag) &&
			 after.c_line == legacy.c_line &&
			 !memcmp(after.c_cc, legacy.c_cc, NCC) &&
			 !memcmp(after.c_cc + NCC, before.c_cc + NCC,
				 NCCS - NCC) &&
			 after.c_ispeed == before.c_ispeed &&
			 after.c_ospeed == before.c_ospeed);

	legacy.c_cflag = (legacy.c_cflag & ~CBAUD) | B9600;
	TEST_SUCC(ioctl(master, TCSETA, &legacy));
	TEST_RES(ioctl(slave, TCGETS2, &after),
		 after.c_ospeed == 9600 && after.c_ispeed == before.c_ispeed);
	TEST_SUCC(ioctl(slave, TCSETS2, &initial));
}
END_TEST()

FN_TEST(legacy_setters_work_on_both_pty_ends)
{
	int fds[] = { master, slave };
	struct termio legacy = { 0 };
	struct termios2 after = { 0 };
	TEST_SUCC(ioctl(slave, TCGETA, &legacy));

	for (size_t i = 0; i < 2; ++i) {
		for (size_t j = 0; j < sizeof(setters) / sizeof(setters[0]);
		     ++j) {
			legacy.c_lflag ^= ECHO;
			TEST_SUCC(ioctl(fds[i], setters[j], &legacy));
			TEST_RES(ioctl(slave, TCGETS2, &after),
				 after.c_lflag == legacy.c_lflag);
		}
	}
}
END_TEST()

FN_TEST(legacy_argument_access)
{
	long page_size = TEST_RES(sysconf(_SC_PAGESIZE), _ret > 0);
	if (page_size <= 0)
		return;
	char *mapping =
		TEST_RES(mmap(NULL, 2 * page_size, PROT_READ | PROT_WRITE,
			      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0),
			 _ret != MAP_FAILED);
	if (mapping == MAP_FAILED)
		return;
	void *argument = mapping + page_size - sizeof(struct termio);

	if (TEST_SUCC(mprotect(mapping + page_size, page_size, PROT_NONE)) <
	    0) {
		TEST_SUCC(munmap(mapping, 2 * page_size));
		return;
	}

	if (TEST_SUCC(ioctl(master, TCGETA, argument)) < 0) {
		TEST_SUCC(munmap(mapping, 2 * page_size));
		return;
	}
	TEST(ioctl(master, TCGETA, (char *)argument + 1), EFAULT, _ret == -1);
	if (TEST_SUCC(ioctl(master, TCGETA, argument)) < 0) {
		TEST_SUCC(munmap(mapping, 2 * page_size));
		return;
	}
	for (size_t j = 0; j < sizeof(setters) / sizeof(setters[0]); ++j) {
		TEST_SUCC(ioctl(master, setters[j], argument));
		TEST(ioctl(master, setters[j], (char *)argument + 1), EFAULT,
		     _ret == -1);
	}
	TEST_SUCC(munmap(mapping, 2 * page_size));
}
END_TEST()

FN_TEST(legacy_flush_discards_input_only_after_valid_argument)
{
	struct termio legacy = { 0 };
	struct pollfd pfd = { .fd = slave, .events = POLLIN };
	int bytes = -1;
	char buffer[16];

	TEST_SUCC(ioctl(slave, TCSETS2, &initial));
	TEST_SUCC(ioctl(master, TCGETA, &legacy));
	legacy.c_lflag &= ~ECHO;
	TEST_SUCC(ioctl(master, TCSETA, &legacy));
	TEST_RES(write(master, "discard\npartial", 15), _ret == 15);
	TEST_RES(poll(&pfd, 1, 1000), _ret == 1 && pfd.revents == POLLIN);
	TEST_RES(ioctl(slave, FIONREAD, &bytes), _ret == 0 && bytes == 8);
	TEST(ioctl(master, TCSETAF, NULL), EFAULT, _ret == -1);
	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 8);
	TEST_SUCC(ioctl(master, TCSETA, &legacy));
	TEST_SUCC(ioctl(master, TCSETAW, &legacy));
	TEST_RES(ioctl(slave, FIONREAD, &bytes), bytes == 8);
	TEST_SUCC(ioctl(master, TCSETAF, &legacy));
	TEST_RES(poll(&pfd, 1, 0), _ret == 0 && pfd.revents == 0);
	TEST(read(slave, buffer, sizeof(buffer)), EAGAIN, _ret == -1);
	// Completing a new line must not expose the discarded partial line.
	TEST_RES(write(master, "\n", 1), _ret == 1);
	TEST_RES(poll(&pfd, 1, 1000), _ret == 1 && pfd.revents == POLLIN);
	TEST_RES(read(slave, buffer, sizeof(buffer)),
		 _ret == 1 && buffer[0] == '\n');
}
END_TEST()

enum io_operation { READ_INPUT, WRITE_INPUT };

// The pipe reports startup and completion without requiring a blocking wait in the parent.
static pid_t start_io_worker(int fd, enum io_operation operation, int *done_fd)
{
	int pipe_fds[2];
	CHECK(pipe(pipe_fds));
	pid_t child = CHECK(fork());
	if (child == 0) {
		char byte = 'x';
		CHECK(close(pipe_fds[0]));
		CHECK(write(pipe_fds[1], &byte, 1));
		ssize_t result = operation == READ_INPUT ? read(fd, &byte, 1) :
							   write(fd, &byte, 1);
		byte = result == 1 && byte == 'x';
		CHECK(write(pipe_fds[1], &byte, 1));
		_exit(0);
	}
	CHECK(close(pipe_fds[1]));
	char byte;
	CHECK_WITH(read(pipe_fds[0], &byte, 1), _ret == 1);
	*done_fd = pipe_fds[0];
	return child;
}

static int finish_io_worker(pid_t child, int done_fd)
{
	struct pollfd pfd = { .fd = done_fd, .events = POLLIN };
	char success = 0;
	int status;
	if (CHECK(poll(&pfd, 1, 2000)) == 1)
		CHECK_WITH(read(done_fd, &success, 1), _ret == 1);
	else
		CHECK(kill(child, SIGKILL));
	CHECK(waitpid(child, &status, 0));
	CHECK(close(done_fd));
	return success && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

FN_TEST(legacy_mode_change_wakes_reader)
{
	TEST_SUCC(fcntl(slave, F_SETFL, 0));
	for (size_t j = 0; j < 2; ++j) {
		struct termios2 canonical = initial;
		canonical.c_lflag &= ~ECHO;
		TEST_SUCC(ioctl(slave, TCSETSF2, &canonical));
		TEST_RES(write(master, "x", 1), _ret == 1);
		struct termio legacy = { 0 };
		TEST_SUCC(ioctl(master, TCGETA, &legacy));
		legacy.c_lflag &= ~ICANON;
		legacy.c_cc[VMIN] = 1;
		legacy.c_cc[VTIME] = 0;

		int done_fd;
		pid_t child = start_io_worker(slave, READ_INPUT, &done_fd);
		struct pollfd pfd = { .fd = done_fd, .events = POLLIN };
		TEST_RES(poll(&pfd, 1, 100), _ret == 0);
		TEST_SUCC(ioctl(master, setters[j], &legacy));
		TEST_RES(finish_io_worker(child, done_fd), _ret == 1);
	}
	TEST_SUCC(fcntl(slave, F_SETFL, O_NONBLOCK));
}
END_TEST()

FN_TEST(legacy_flush_wakes_writer)
{
#ifndef __asterinas__
	// Linux has an additional asynchronous input queue, so a single flush need not release a writer.
	SKIP_TEST_IF(1);
#endif
	struct termios2 raw = initial;
	raw.c_iflag = 0;
	raw.c_oflag = 0;
	raw.c_lflag = 0;
	raw.c_cc[VMIN] = 1;
	raw.c_cc[VTIME] = 0;
	TEST_SUCC(ioctl(slave, TCSETSF2, &raw));
	TEST_SUCC(fcntl(master, F_SETFL, O_NONBLOCK));
	char buffer[4096];
	memset(buffer, 'x', sizeof(buffer));
	size_t total = 0;
	// Asterinas pushes input synchronously; EAGAIN means the line discipline is full.
	for (;;) {
		ssize_t size = write(master, buffer, sizeof(buffer));
		if (size < 0) {
			int write_errno = errno;
			TEST_RES(write_errno, _ret == EAGAIN);
			break;
		} else {
			total += size;
			if (total >= 1024 * 1024) {
				TEST_RES(total, _ret < 1024 * 1024);
				break;
			}
		}
	}
	TEST_SUCC(fcntl(master, F_SETFL, 0));
	struct termio legacy = { 0 };
	TEST_SUCC(ioctl(slave, TCGETA, &legacy));
	int done_fd;
	pid_t child = start_io_worker(master, WRITE_INPUT, &done_fd);
	struct pollfd pfd = { .fd = done_fd, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, 100), _ret == 0);
	TEST_SUCC(ioctl(slave, TCSETAF, &legacy));
	TEST_RES(finish_io_worker(child, done_fd), _ret == 1);
}
END_TEST()

FN_TEST(legacy_flush_notifies_packet_reader)
{
	struct termios2 termios = initial;
	termios.c_lflag &= ~ECHO;
	TEST_SUCC(ioctl(slave, TCSETSF2, &termios));
	TEST_SUCC(fcntl(master, F_SETFL, O_NONBLOCK));
	int packet_mode = 1;
	TEST_SUCC(ioctl(master, TIOCPKT, &packet_mode));
	struct termio legacy = { 0 };
	TEST_SUCC(ioctl(master, TCGETA, &legacy));
	struct pollfd pfd = { .fd = master, .events = POLLIN | POLLPRI };
	TEST(ioctl(master, TCSETAF, NULL), EFAULT, _ret == -1);
	TEST_SUCC(ioctl(master, TCSETA, &legacy));
	TEST_SUCC(ioctl(master, TCSETAW, &legacy));
	TEST_RES(poll(&pfd, 1, 0), _ret == 0);
	TEST_SUCC(ioctl(master, TCSETAF, &legacy));
	TEST_RES(poll(&pfd, 1, 1000),
		 _ret == 1 && pfd.revents == (POLLIN | POLLPRI));
	unsigned char packet[16];
	TEST_RES(read(master, packet, sizeof(packet)),
		 _ret == 1 && packet[0] == TIOCPKT_FLUSHREAD);
	packet_mode = 0;
	TEST_SUCC(ioctl(master, TIOCPKT, &packet_mode));
	TEST_SUCC(fcntl(master, F_SETFL, 0));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(master));
	CHECK(close(slave));
}
END_SETUP()
