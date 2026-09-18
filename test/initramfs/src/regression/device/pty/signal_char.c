// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <poll.h>
#include <pty.h>
#include <signal.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>
#include "../../common/test.h"

static int master, slave;

static volatile sig_atomic_t nr_sigint, nr_sigquit, nr_sigtstp;

static void count_signal(int signum)
{
	switch (signum) {
	case SIGINT:
		nr_sigint++;
		break;
	case SIGQUIT:
		nr_sigquit++;
		break;
	case SIGTSTP:
		nr_sigtstp++;
		break;
	}
}

FN_SETUP(openpty)
{
	CHECK(openpty(&master, &slave, NULL, NULL, NULL));

	CHECK(fcntl(master, F_SETFL, O_NONBLOCK));
	CHECK(fcntl(slave, F_SETFL, O_NONBLOCK));
}
END_SETUP()

FN_SETUP(new_session)
{
	int status;

	if (CHECK(fork()) != 0) {
		CHECK_WITH(wait(&status),
			   WIFEXITED(status) && WEXITSTATUS(status) == 0);
		exit(EXIT_SUCCESS);
	}

	CHECK(setsid());
	CHECK(ioctl(slave, TIOCSCTTY, 0));
}
END_SETUP()

FN_SETUP(signal_handlers)
{
	struct sigaction sa = { .sa_handler = count_signal,
				.sa_flags = SA_RESTART };

	CHECK(sigaction(SIGINT, &sa, NULL));
	CHECK(sigaction(SIGQUIT, &sa, NULL));
	CHECK(sigaction(SIGTSTP, &sa, NULL));
}
END_SETUP()

static void set_lflags(tcflag_t lflags)
{
	struct termios termios;

	CHECK(tcgetattr(slave, &termios));
	termios.c_lflag = lflags;
	termios.c_cc[VMIN] = 1;
	termios.c_cc[VTIME] = 0;
	CHECK(tcsetattr(slave, TCSANOW, &termios));
}

static void reset_state(void)
{
	char buf[64];

	while (read(slave, buf, sizeof(buf)) > 0)
		;
	while (read(master, buf, sizeof(buf)) > 0)
		;

	nr_sigint = nr_sigquit = nr_sigtstp = 0;
}

// Returns 1 if the signal counter becomes positive before the timeout.
static int wait_sig(volatile sig_atomic_t *counter)
{
	int i;

	for (i = 0; i < 200 && *counter == 0; i++)
		usleep(10 * 1000);

	// `usleep` may have been interrupted by the signal we are waiting
	// for. This is expected, so do not let `EINTR` leak to the caller.
	errno = 0;
	return *counter > 0;
}

// PTY data flow is asynchronous (see pty(7)): bytes written to one end may
// not be readable at the other end immediately. Returns 1 if the file
// descriptor becomes readable before the timeout.
static int poll_in_timeout(int fd, int timeout_ms)
{
	struct pollfd pfd = { .fd = fd, .events = POLLIN };

	return poll(&pfd, 1, timeout_ms) == 1 && (pfd.revents & POLLIN) != 0;
}

static int poll_in(int fd)
{
	return poll_in_timeout(fd, 2000);
}

// Reads until `len` bytes have arrived or the timeout expires. Returns the
// number of bytes read.
static int read_upto(int fd, char *buf, int len)
{
	int total = 0, ret;

	while (total < len && poll_in(fd)) {
		ret = read(fd, buf + total, len - total);
		if (ret <= 0)
			break;
		total += ret;
	}

	errno = 0;
	return total;
}

// Reads and discards everything until the file descriptor stays quiet.
static void drain(int fd)
{
	char buf[4096];

	while (poll_in_timeout(fd, 200) && read(fd, buf, sizeof(buf)) > 0)
		;

	errno = 0;
}

// Signal characters must be recognized with ISIG alone; canonical mode is
// not required (e.g., shells keep ISIG on while readline puts the terminal
// in non-canonical mode). The character itself must be eaten by the line
// discipline instead of being delivered to readers.
FN_TEST(raw_isig_signal_chars)
{
	char buf[8];

	set_lflags(ISIG);
	reset_state();

	TEST_RES(write(master, "\x03", 1), _ret == 1);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);
	TEST_ERRNO(read(slave, buf, sizeof(buf)), EAGAIN);

	TEST_RES(write(master, "\x1c", 1), _ret == 1);
	TEST_RES(wait_sig(&nr_sigquit), _ret == 1);
	TEST_ERRNO(read(slave, buf, sizeof(buf)), EAGAIN);

	TEST_RES(write(master, "\x1a", 1), _ret == 1);
	TEST_RES(wait_sig(&nr_sigtstp), _ret == 1);
	TEST_ERRNO(read(slave, buf, sizeof(buf)), EAGAIN);
}
END_TEST()

// Without NOFLSH, a signal character discards pending input.
FN_TEST(signal_char_flushes_input)
{
	char buf[8];

	set_lflags(ISIG);
	reset_state();

	TEST_RES(write(master, "ab\x03", 3), _ret == 3);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);
	TEST_ERRNO(read(slave, buf, sizeof(buf)), EAGAIN);
}
END_TEST()

// With NOFLSH, pending input survives, but the signal character is still
// consumed.
FN_TEST(noflsh_keeps_input)
{
	char buf[8];

	set_lflags(ISIG | NOFLSH);
	reset_state();

	TEST_RES(write(master, "cd\x03", 3), _ret == 3);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);
	TEST_RES(poll_in(slave), _ret == 1);
	TEST_RES(read(slave, buf, sizeof(buf)),
		 _ret == 2 && buf[0] == 'c' && buf[1] == 'd');
	TEST_ERRNO(read(slave, buf, sizeof(buf)), EAGAIN);
}
END_TEST()

// Without NOFLSH, a signal character also discards the echoes of the input
// that precedes it in the same batch. Its own echo comes after the flush.
FN_TEST(signal_char_flushes_echoes)
{
	char buf[8];

	set_lflags(ISIG | ECHO | ECHOCTL);
	reset_state();

	TEST_RES(write(master, "ab\x03", 3), _ret == 3);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);
	TEST_RES(read_upto(master, buf, 2),
		 _ret == 2 && buf[0] == '^' && buf[1] == 'C');
	TEST_ERRNO(read(master, buf, sizeof(buf)), EAGAIN);
	TEST_ERRNO(read(slave, buf, sizeof(buf)), EAGAIN);
}
END_TEST()

// With NOFLSH, the pending echoes survive and keep their order.
FN_TEST(noflsh_keeps_echoes)
{
	char buf[8];

	set_lflags(ISIG | ECHO | ECHOCTL | NOFLSH);
	reset_state();

	TEST_RES(write(master, "cd\x03", 3), _ret == 3);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);
	TEST_RES(read_upto(master, buf, 4),
		 _ret == 4 && memcmp(buf, "cd^C", 4) == 0);
	TEST_ERRNO(read(master, buf, sizeof(buf)), EAGAIN);
	TEST_RES(poll_in(slave), _ret == 1);
	TEST_RES(read(slave, buf, sizeof(buf)),
		 _ret == 2 && buf[0] == 'c' && buf[1] == 'd');
}
END_TEST()

// An echo that does not fit into the full output buffer is not lost: it is
// written out before the next output.
FN_TEST(echo_deferred_by_full_output)
{
	static char big[4096];
	char buf[8];

	set_lflags(ISIG | ECHO | ECHOCTL);
	reset_state();

	// Fill the output buffer without reading from the master. Bytes may still
	// be in flight towards the master (see pty(7)), so retry until the buffer
	// stays full.
	memset(big, 'a', sizeof(big));
	do {
		while (write(slave, big, sizeof(big)) > 0)
			;
		usleep(50 * 1000);
	} while (write(slave, big, sizeof(big)) > 0);
	TEST_ERRNO(write(slave, big, sizeof(big)), EAGAIN);

	TEST_RES(write(master, "x", 1), _ret == 1);
	drain(master);

	TEST_RES(write(slave, "y", 1), _ret == 1);
	TEST_RES(read_upto(master, buf, 2),
		 _ret == 2 && buf[0] == 'x' && buf[1] == 'y');
	TEST_ERRNO(read(master, buf, sizeof(buf)), EAGAIN);
}
END_TEST()

// A batch whose echoes are twice its size (ECHOCTL doubles every control
// character) is echoed in full when the output buffer has room.
FN_TEST(echo_batch_doubled_by_echoctl)
{
	static char big[3000], out[6000];
	char buf[8];
	int i;

	set_lflags(ISIG | ECHO | ECHOCTL);
	reset_state();

	memset(big, '\x01', sizeof(big));
	TEST_RES(write(master, big, sizeof(big)), _ret == (int)sizeof(big));
	TEST_RES(read_upto(master, out, sizeof(out)), _ret == (int)sizeof(out));
	for (i = 0; i < (int)sizeof(out); i += 2)
		if (out[i] != '^' || out[i + 1] != 'A')
			break;
	TEST_RES(i, _ret == (int)sizeof(out));
	TEST_ERRNO(read(master, buf, sizeof(buf)), EAGAIN);
}
END_TEST()

#ifdef __asterinas__
// An echo unit is never split across commits: with one byte free in the
// output ring, `^C` waits as a whole, so a later flush cannot leave a stray
// `^` behind. The setup relies on the 8 KiB pty output ring, so this case
// cannot run on Linux.
FN_TEST(echo_unit_not_split)
{
	static char big[8191];
	char buf[8];

	set_lflags(ISIG | ECHO | ECHOCTL);
	reset_state();

	memset(big, 'a', sizeof(big));
	TEST_RES(write(slave, big, 4096), _ret == 4096);
	TEST_RES(write(slave, big, 4095), _ret == 4095);

	TEST_RES(write(master, "\x03", 1), _ret == 1);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);
	nr_sigint = 0;
	TEST_RES(write(master, "\x03", 1), _ret == 1);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);

	TEST_RES(read_upto(master, big, sizeof(big)),
		 _ret == (int)sizeof(big) && big[sizeof(big) - 1] == 'a');
	TEST_ERRNO(read(master, buf, sizeof(buf)), EAGAIN);

	TEST_RES(write(slave, "y", 1), _ret == 1);
	TEST_RES(read_upto(master, buf, 3),
		 _ret == 3 && memcmp(buf, "^Cy", 3) == 0);
	TEST_ERRNO(read(master, buf, sizeof(buf)), EAGAIN);
}
END_TEST()
#endif

// Without ISIG, signal characters are ordinary input.
FN_TEST(isig_off_passes_through)
{
	char buf[8];

	set_lflags(0);
	reset_state();

	TEST_RES(write(master, "\x03", 1), _ret == 1);
	TEST_RES(poll_in(slave), _ret == 1);
	TEST_RES(read(slave, buf, sizeof(buf)), _ret == 1 && buf[0] == '\x03');

	usleep(100 * 1000);
	TEST_RES((int)nr_sigint, _ret == 0);
}
END_TEST()

// In canonical mode, a signal character also discards the unfinished line.
FN_TEST(canonical_isig_flushes_line)
{
	char buf[8];

	set_lflags(ICANON | ISIG);
	reset_state();

	TEST_RES(write(master, "ab\x03", 3), _ret == 3);
	TEST_RES(wait_sig(&nr_sigint), _ret == 1);

	TEST_RES(write(master, "\n", 1), _ret == 1);
	TEST_RES(poll_in(slave), _ret == 1);
	TEST_RES(read(slave, buf, sizeof(buf)), _ret == 1 && buf[0] == '\n');
}
END_TEST()

// A special character whose value is `_POSIX_VDISABLE` is disabled, so a NUL
// byte must stay ordinary input. Keep this last: it leaves VINTR disabled.
FN_TEST(disabled_intr_char)
{
	struct termios termios;
	char buf[8];

	set_lflags(ISIG);
	CHECK(tcgetattr(slave, &termios));
	termios.c_cc[VINTR] = _POSIX_VDISABLE;
	CHECK(tcsetattr(slave, TCSANOW, &termios));
	reset_state();

	TEST_RES(write(master, "\0", 1), _ret == 1);
	TEST_RES(poll_in(slave), _ret == 1);
	TEST_RES(read(slave, buf, sizeof(buf)), _ret == 1 && buf[0] == '\0');

	usleep(100 * 1000);
	TEST_RES((int)nr_sigint, _ret == 0);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(slave));
	CHECK(close(master));
}
END_SETUP()
