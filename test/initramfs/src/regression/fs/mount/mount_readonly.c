// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <sched.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define SOURCE "/tmp/readonly_source"
#define ALIAS "/tmp/readonly_alias"
#define CONTENT "ORIGINAL"

// Regression for #4088: existing files on a read-only mount must not be
// writable, even when their inode permissions allow writes.
FN_SETUP(mount_fixtures)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mount(NULL, "/", NULL, MS_PRIVATE | MS_REC, NULL));
	CHECK(mkdir(SOURCE, 0755));
	CHECK(mkdir(ALIAS, 0755));
	CHECK(mount("tmpfs", SOURCE, "tmpfs", 0, NULL));
	int fd = CHECK(open(SOURCE "/file", O_CREAT | O_RDWR, 0666));
	CHECK_WITH(write(fd, CONTENT, sizeof(CONTENT) - 1),
		   _ret == sizeof(CONTENT) - 1);
	CHECK(close(fd));
	CHECK(chmod(SOURCE "/file", 0666));
	CHECK(mkfifo(SOURCE "/fifo", 0666));
	CHECK(mknod(SOURCE "/null", S_IFCHR | 0666, makedev(1, 3)));
	CHECK(mount(SOURCE, ALIAS, NULL, MS_BIND, NULL));
	CHECK(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY, NULL));
}
END_SETUP()

FN_TEST(readonly_bind_preserves_writable_source)
{
	TEST_ERRNO(open(ALIAS "/file", O_WRONLY), EROFS);
	int fd = TEST_SUCC(open(SOURCE "/file", O_RDWR));
	TEST_RES(pwrite(fd, CONTENT, sizeof(CONTENT) - 1, 0),
		 _ret == sizeof(CONTENT) - 1);
	TEST_SUCC(ftruncate(fd, sizeof(CONTENT) - 1));
	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(remount_source_readonly)
{
	CHECK(mount(NULL, SOURCE, NULL, MS_REMOUNT | MS_RDONLY, NULL));
}
END_SETUP()

FN_TEST(readonly_rejects_writable_opens_and_truncate)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		TEST_SUCC(setgid(1000));
		TEST_SUCC(setuid(1000));
		const int flags[] = {
			O_WRONLY,
			O_RDWR,
			O_WRONLY | O_APPEND,
			O_WRONLY | O_CREAT,
			O_WRONLY | O_TRUNC,
			O_RDWR | O_TRUNC,
			O_RDONLY | O_TRUNC,
		};
		for (size_t i = 0; i < sizeof(flags) / sizeof(flags[0]); i++) {
			TEST_ERRNO(open(SOURCE "/file", flags[i], 0666), EROFS);
		}
		TEST_ERRNO(truncate(SOURCE "/file", 0), EROFS);
		TEST_ERRNO(truncate(SOURCE "/file", sizeof(CONTENT) - 1),
			   EROFS);

		int fd = TEST_SUCC(open(SOURCE "/file", O_RDONLY));
		char buf[sizeof(CONTENT)] = { 0 };
		TEST_RES(read(fd, buf, sizeof(buf)),
			 _ret == sizeof(CONTENT) - 1);
		TEST_RES(memcmp(buf, CONTENT, sizeof(CONTENT) - 1), _ret == 0);
		struct stat st = { 0 };
		TEST_SUCC(fstat(fd, &st));
		TEST_RES(st.st_size, st.st_size == sizeof(CONTENT) - 1);
		TEST_SUCC(close(fd));
		_exit(__tests_failed == 0 ? EXIT_SUCCESS : EXIT_FAILURE);
	}
	int status = 0;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(readonly_allows_path_and_special_file_opens)
{
	int fd = TEST_SUCC(open(SOURCE "/file", O_PATH | O_RDWR | O_TRUNC));
	TEST_SUCC(close(fd));
	fd = TEST_SUCC(open(SOURCE "/file", O_RDONLY | O_CREAT, 0666));
	TEST_SUCC(close(fd));
	fd = TEST_SUCC(open(SOURCE, O_RDONLY | O_DIRECTORY));
	TEST_SUCC(close(fd));
	TEST_ERRNO(open(SOURCE, O_WRONLY), EISDIR);
	fd = TEST_SUCC(open(SOURCE "/fifo", O_RDWR | O_NONBLOCK));
	TEST_SUCC(close(fd));
	fd = TEST_SUCC(open(SOURCE "/null", O_WRONLY));
	TEST_RES(write(fd, "x", 1), _ret == 1);
	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(remove_fixtures)
{
	CHECK(umount(ALIAS));
	CHECK(mount(NULL, SOURCE, NULL, MS_REMOUNT, NULL));
	CHECK(unlink(SOURCE "/file"));
	CHECK(unlink(SOURCE "/fifo"));
	CHECK(unlink(SOURCE "/null"));
	CHECK(umount(SOURCE));
	CHECK(rmdir(ALIAS));
	CHECK(rmdir(SOURCE));
}
END_SETUP()
