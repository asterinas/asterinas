// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <linux/close_range.h>
#include <sys/inotify.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_DIR "/tmp/inotify_close"
#define CLOSE_RANGE_FILE TEST_DIR "/close_range_file"
#define CLOSE_RANGE_NAME "close_range_file"
#define DUP2_FILE TEST_DIR "/dup2_file"
#define DUP2_NAME "dup2_file"

static int inotify_fd;

FN_SETUP(init)
{
	CHECK(mkdir(TEST_DIR, 0700));
	inotify_fd = CHECK(inotify_init1(O_NONBLOCK));
	CHECK(inotify_add_watch(inotify_fd, TEST_DIR, IN_CLOSE_NOWRITE));
}
END_SETUP()

// Reads the next inotify event and accepts only the close event that the
// test expects
static int read_close_event(const char *name)
{
	char buf[sizeof(struct inotify_event) + 64]
		__attribute__((aligned(__alignof__(struct inotify_event))));
	struct inotify_event *event = (struct inotify_event *)buf;

	ssize_t len = read(inotify_fd, buf, sizeof(buf));
	if (len < (ssize_t)sizeof(*event)) {
		return -1;
	}

	if (event->mask != IN_CLOSE_NOWRITE) {
		return -1;
	}

	if (strcmp(event->name, name) != 0) {
		return -1;
	}

	return 0;
}

FN_TEST(close_range_reports_close)
{
	int fd = TEST_SUCC(open(CLOSE_RANGE_FILE, O_CREAT | O_RDONLY, 0600));

	TEST_SUCC(close_range(fd, fd, 0));
	TEST_RES(read_close_event(CLOSE_RANGE_NAME), _ret == 0);
}
END_TEST()

FN_TEST(dup2_reports_replaced_fd_close)
{
	int old_fd = TEST_SUCC(open("/dev/null", O_RDONLY));
	int new_fd = TEST_SUCC(open(DUP2_FILE, O_CREAT | O_RDONLY, 0600));

	TEST_RES(dup2(old_fd, new_fd), _ret == new_fd);
	TEST_RES(read_close_event(DUP2_NAME), _ret == 0);
	TEST_SUCC(close(old_fd));
	TEST_SUCC(close(new_fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(inotify_fd));
	CHECK(unlink(CLOSE_RANGE_FILE));
	CHECK(unlink(DUP2_FILE));
	CHECK(rmdir(TEST_DIR));
}
END_SETUP()
