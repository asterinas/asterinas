// SPDX-License-Identifier: MPL-2.0

#include <sys/inotify.h>
#include <sys/fcntl.h>
#include <unistd.h>
#include "../../common/test.h"

#define TEST_FILE "/tmp/test1"

FN_TEST(unlink_add)
{
	int inotify_fd, fd, wd;

	inotify_fd = TEST_SUCC(inotify_init1(O_NONBLOCK));

	fd = TEST_RES(open(TEST_FILE, O_CREAT | O_WRONLY, 0644), _ret == 4);
	TEST_SUCC(unlink(TEST_FILE));

	// FIXME: Asterinas currently does not support adding inotify watches
	// to deleted inodes.
#ifdef __asterinas__
	TEST_ERRNO(inotify_add_watch(inotify_fd, "/proc/self/fd/4",
				     IN_DELETE_SELF),
		   ENOENT);
	(void)wd;
#else
	wd = TEST_SUCC(inotify_add_watch(inotify_fd, "/proc/self/fd/4",
					 IN_DELETE_SELF));
	TEST_SUCC(inotify_rm_watch(inotify_fd, wd));
#endif

	TEST_SUCC(close(fd));
	TEST_SUCC(close(inotify_fd));
}
END_TEST()
