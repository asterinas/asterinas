// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <poll.h>
#include <sys/inotify.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/tmp"
#define TEST_FILE "/tmp/inotify_o_path_test"

/*
 * Verify that opening a file with `O_PATH` does not generate inotify events.
 *
 * Linux sets `FMODE_NONOTIFY` on `O_PATH` file descriptors, so `IN_OPEN`
 * and `IN_CLOSE_NOWRITE` must not be reported to a watch on the parent
 * directory.
 */
FN_TEST(o_path_open_suppresses_events)
{
	int ifd, wd, fd;
	struct pollfd pfd;

	/* Create the target file. */
	fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	/* Set up inotify on the parent directory. */
	ifd = TEST_SUCC(inotify_init1(IN_NONBLOCK));
	wd = TEST_SUCC(
		inotify_add_watch(ifd, TEST_DIR, IN_OPEN | IN_CLOSE_NOWRITE));

	/* Open with O_PATH — this must NOT generate any event. */
	fd = TEST_SUCC(open(TEST_FILE, O_PATH));
	TEST_SUCC(close(fd));

	/* Poll the inotify fd; no events should be pending. */
	pfd.fd = ifd;
	pfd.events = POLLIN;
	TEST_RES(poll(&pfd, 1, 0), _ret == 0);

	TEST_SUCC(inotify_rm_watch(ifd, wd));
	TEST_SUCC(close(ifd));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()

/*
 * Verify that a normal open (without `O_PATH`) still generates events,
 * as a sanity check that inotify itself works correctly.
 */
FN_TEST(normal_open_generates_events)
{
	int ifd, wd, fd;
	struct pollfd pfd;

	/* Create the target file. */
	fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	/* Set up inotify on the parent directory. */
	ifd = TEST_SUCC(inotify_init1(IN_NONBLOCK));
	wd = TEST_SUCC(
		inotify_add_watch(ifd, TEST_DIR, IN_OPEN | IN_CLOSE_NOWRITE));

	/* Normal open — this MUST generate IN_OPEN. */
	fd = TEST_SUCC(open(TEST_FILE, O_RDONLY));
	TEST_SUCC(close(fd));

	/* Poll the inotify fd; events should be pending. */
	pfd.fd = ifd;
	pfd.events = POLLIN;
	TEST_RES(poll(&pfd, 1, 0), _ret == 1);

	TEST_SUCC(inotify_rm_watch(ifd, wd));
	TEST_SUCC(close(ifd));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()
