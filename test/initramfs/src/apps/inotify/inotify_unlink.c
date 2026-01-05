// SPDX-License-Identifier: MPL-2.0

#include <sys/inotify.h>
#include <sys/fcntl.h>
#include <unistd.h>
#include "../test.h"

#define TEST_FILE "/tmp/test1"

FN_TEST(unlink_add)
{
	int inotify_fd, fd, wd;

	inotify_fd = TEST_SUCC(inotify_init1(O_NONBLOCK));

	fd = TEST_RES(open(TEST_FILE, O_CREAT | O_WRONLY, 0644), _ret == 4);
	TEST_SUCC(unlink(TEST_FILE));

	wd = TEST_SUCC(inotify_add_watch(inotify_fd, "/proc/self/fd/4",
					 IN_DELETE_SELF));
	TEST_SUCC(inotify_rm_watch(inotify_fd, wd));

	TEST_SUCC(close(fd));
	TEST_SUCC(close(inotify_fd));
}
END_TEST()

FN_TEST(unlink_closed)
{
	int inotify_fd, fd, wd;
	struct inotify_event ev;

	inotify_fd = TEST_SUCC(inotify_init1(O_NONBLOCK));

	fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_WRONLY, 0644));
	wd = TEST_SUCC(
		inotify_add_watch(inotify_fd, TEST_FILE, IN_DELETE_SELF));

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));

	// Both `IN_DELETE_SELF` and `IN_IGNORED` are generated.
	TEST_RES(read(inotify_fd, &ev, sizeof(ev)),
		 _ret == sizeof(ev) && ev.mask == IN_DELETE_SELF);
	TEST_RES(read(inotify_fd, &ev, sizeof(ev)),
		 _ret == sizeof(ev) && ev.mask == IN_IGNORED);
	TEST_ERRNO(read(inotify_fd, &ev, sizeof(ev)), EAGAIN);

	// The watch does not exist anymore.
	TEST_ERRNO(inotify_rm_watch(inotify_fd, wd), EINVAL);

	TEST_SUCC(close(inotify_fd));
}
END_TEST()

FN_TEST(unlink_remove)
{
	int inotify_fd1, inotify_fd2, fd, wd1, wd2;
	struct inotify_event ev;

	inotify_fd1 = TEST_SUCC(inotify_init1(O_NONBLOCK));
	inotify_fd2 = TEST_SUCC(inotify_init1(O_NONBLOCK));

	fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_WRONLY, 0644));
	wd1 = TEST_SUCC(
		inotify_add_watch(inotify_fd1, TEST_FILE, IN_DELETE_SELF));
	wd2 = TEST_SUCC(
		inotify_add_watch(inotify_fd2, TEST_FILE, IN_DELETE_SELF));

	TEST_SUCC(unlink(TEST_FILE));
	TEST_ERRNO(read(inotify_fd1, &ev, sizeof(ev)), EAGAIN);
	TEST_ERRNO(read(inotify_fd2, &ev, sizeof(ev)), EAGAIN);

	// Removing the watch after unlinking is fine.
	TEST_SUCC(inotify_rm_watch(inotify_fd2, wd2));
	TEST_RES(read(inotify_fd2, &ev, sizeof(ev)),
		 _ret == sizeof(ev) && ev.mask == IN_IGNORED);
	TEST_ERRNO(read(inotify_fd2, &ev, sizeof(ev)), EAGAIN);

	// `IN_DELETE_SELF` and `IN_IGNORED` will be generated once we close `fd`.
	TEST_SUCC(close(fd));
	TEST_RES(read(inotify_fd1, &ev, sizeof(ev)),
		 _ret == sizeof(ev) && ev.mask == IN_DELETE_SELF);
	TEST_RES(read(inotify_fd1, &ev, sizeof(ev)),
		 _ret == sizeof(ev) && ev.mask == IN_IGNORED);
	TEST_ERRNO(read(inotify_fd1, &ev, sizeof(ev)), EAGAIN);

	// The watch does not exist anymore after we close `fd`.
	TEST_ERRNO(inotify_rm_watch(inotify_fd1, wd1), EINVAL);

	TEST_SUCC(close(inotify_fd1));
	TEST_SUCC(close(inotify_fd2));
}
END_TEST()
