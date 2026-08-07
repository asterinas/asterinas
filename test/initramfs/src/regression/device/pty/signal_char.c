// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <poll.h>
#include <pty.h>
#include <signal.h>
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
static int poll_in(int fd)
{
	struct pollfd pfd = { .fd = fd, .events = POLLIN };

	return poll(&pfd, 1, 2000) == 1 && (pfd.revents & POLLIN) != 0;
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
