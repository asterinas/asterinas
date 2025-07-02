// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <pthread.h>
#include <pty.h>
#include <unistd.h>
#include <sys/ioctl.h>

static int master, slave;

FN_SETUP(openpty)
{
	CHECK(openpty(&master, &slave, NULL, NULL, NULL));
}
END_SETUP()

static int write_repeat(int fd, char c, size_t n)
{
	while (n--)
		if (write(fd, &c, 1) < 0)
			return -1;

	return 0;
}

static int read_repeat(int fd, char c, size_t n)
{
	char c2;

	while (n--)
		if (read(fd, &c2, 1) != 1 || c2 != c)
			return -1;

	return 0;
}

static int read_all(int fd, char c)
{
	int n;

	for (;;) {
		if (ioctl(fd, FIONREAD, &n) < 0)
			return -1;

		if (n == 0)
			return 0;

		if (read_repeat(fd, c, n) < 0)
			return -1;
	}
}

#define PTR_VOID_FALSE ((void *)0)
#define PTR_VOID_TRUE ((void *)1)

// Tests that `block` will block the thread until `unblock` is called.
#define DECLARE_BLOCKING_TEST(name, block, unblock)    \
	void *blocking_##name(void *is_child)          \
	{                                              \
		static _Atomic int started = 0;        \
		static _Atomic int ended = 0;          \
                                                       \
		if (!is_child) {                       \
			started = 1;                   \
			block;                         \
                                                       \
			if (ended != 1)                \
				return PTR_VOID_FALSE; \
		} else {                               \
			usleep(100 * 1000);            \
			if (started != 1)              \
				return PTR_VOID_FALSE; \
                                                       \
			ended = 1;                     \
			unblock;                       \
		}                                      \
                                                       \
		return PTR_VOID_TRUE;                  \
	}
#define RUN_BLOCKING_TEST(name)                                                \
	{                                                                      \
		pthread_t __thd;                                               \
		void *__res;                                                   \
		TEST_SUCC(pthread_create(&__thd, NULL, blocking_##name,        \
					 PTR_VOID_TRUE));                      \
		TEST_SUCC(blocking_##name(PTR_VOID_FALSE) == PTR_VOID_TRUE ?   \
				  0 :                                          \
				  -1);                                         \
		TEST_RES(pthread_join(__thd, &__res), __res == PTR_VOID_TRUE); \
	}

DECLARE_BLOCKING_TEST(write_slave, write_repeat(slave, 'a', 1),
		      read_all(master, 'a'));
DECLARE_BLOCKING_TEST(read_master, read_repeat(master, 'a', 1),
		      write_repeat(slave, 'a', 1));
DECLARE_BLOCKING_TEST(read_slave, read_repeat(slave, 'a', 1),
		      write_repeat(master, '\n', 1));

FN_TEST(pty_blocking)
{
	// Write many characters to overflow the line buffer. As documented in the man pages, the
	// line buffer can hold a maximum of 4095 characters, not including the line terminator.
	// Additional characters will not be queued, but signals and echoes will still work.
	// Therefore, the echoed characters will cause the output buffer to overflow.
	TEST_SUCC(write_repeat(master, 'a', 128 * 1024));

	// Since the output buffer is overflowing, writing characters to it should block. In Linux,
	// reading one character from the buffer does not unblock the writer, which is rather odd.
	// So here we test that reading all characters from the buffer should unblock the printer.
	RUN_BLOCKING_TEST(write_slave);
	TEST_SUCC(read_all(master, 'a'));

	// Now that the output buffer is empty, reading characters from it should block. Writing a
	// character to the buffer should unblock the reader.
	RUN_BLOCKING_TEST(read_master);

	// The input buffer is empty because all characters are in the line buffer until a line
	// terminator is seen. So reading characters from the input buffer should block. Writing a
	// line character should move all characters in the line buffer into the input buffer and
	// unblock the reader.
	RUN_BLOCKING_TEST(read_slave);
	TEST_SUCC(read_repeat(slave, 'a', 4094));
	TEST_SUCC(read_repeat(slave, '\n', 1));

	// TODO: This test does not cover cases in which the input buffer overflows. Reliably
	// constructing that state is difficult without first knowing the size of the input buffer.
}
END_TEST()
