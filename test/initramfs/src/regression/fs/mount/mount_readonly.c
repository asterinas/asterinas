// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <sched.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/test.h"

#define SOURCE "/tmp/readonly_source"
#define ALIAS "/tmp/readonly_alias"
#define OTHER_ALIAS "/tmp/readonly_other_alias"
#define CONTENT "ORIGINAL"
#define XATTR_NAME "user.test"
#define XATTR_VALUE "old"

FN_SETUP(mount_fixtures)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mount(NULL, "/", NULL, MS_PRIVATE | MS_REC, NULL));

	CHECK(mkdir(SOURCE, 0755));
	CHECK(mount("tmpfs", SOURCE, "tmpfs", 0, NULL));

	int fd = CHECK(open(SOURCE "/file", O_CREAT | O_RDWR, 0666));
	CHECK(fchmod(fd, 0666));
	CHECK_WITH(write(fd, CONTENT, sizeof(CONTENT) - 1),
		   _ret == sizeof(CONTENT) - 1);
	CHECK(close(fd));
	CHECK(setxattr(SOURCE "/file", XATTR_NAME, XATTR_VALUE,
		       sizeof(XATTR_VALUE) - 1, 0));

	CHECK(mkfifo(SOURCE "/fifo", 0666));
	CHECK(mknod(SOURCE "/null", S_IFCHR | 0666, makedev(1, 3)));

	CHECK(mkdir(ALIAS, 0755));
	CHECK(mount(SOURCE, ALIAS, NULL, MS_BIND, NULL));
	CHECK(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY, NULL));
}
END_SETUP()

// A writable file description prevents a read-only bind remount.
FN_TEST(readonly_bind_remount_rejects_writable_opens)
{
	const int flags[] = { O_WRONLY, O_RDWR, O_WRONLY | O_APPEND };

	for (size_t i = 0; i < sizeof(flags) / sizeof(flags[0]); i++) {
		TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND, NULL));

		int fd = TEST_SUCC(open(ALIAS "/file", flags[i]));

		TEST_ERRNO(mount(NULL, ALIAS, NULL,
				 MS_REMOUNT | MS_BIND | MS_RDONLY, NULL),
			   EBUSY);

		// A failed remount must leave the mount writable.
		int another_fd = TEST_SUCC(open(ALIAS "/file", O_WRONLY));
		TEST_SUCC(close(another_fd));

		TEST_SUCC(close(fd));

		TEST_SUCC(mount(NULL, ALIAS, NULL,
				MS_REMOUNT | MS_BIND | MS_RDONLY, NULL));
	}
}
END_TEST()

FN_TEST(duplicated_fd_keeps_mount_busy)
{
	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND, NULL));

	int fd = TEST_SUCC(open(ALIAS "/file", O_RDWR));
	int duplicate = TEST_SUCC(dup(fd));

	TEST_SUCC(close(fd));

	TEST_ERRNO(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			 NULL),
		   EBUSY);

	TEST_SUCC(close(duplicate));

	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			NULL));
}
END_TEST()

FN_TEST(inherited_fd_keeps_mount_busy)
{
	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND, NULL));

	int fd = TEST_SUCC(open(ALIAS "/file", O_RDWR));
	int ready[2], release[2];
	TEST_SUCC(pipe(ready));
	TEST_SUCC(pipe(release));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(close(ready[0]));
		CHECK(close(release[1]));

		char token = 'x';
		CHECK_WITH(write(ready[1], &token, 1), _ret == 1);
		CHECK_WITH(read(release[0], &token, 1), _ret == 1);

		CHECK(close(fd));
		CHECK(close(ready[1]));
		CHECK(close(release[0]));

		_exit(EXIT_SUCCESS);
	}

	TEST_SUCC(close(ready[1]));
	TEST_SUCC(close(release[0]));
	TEST_SUCC(close(fd));

	char token;
	TEST_RES(read(ready[0], &token, 1), _ret == 1);

	TEST_ERRNO(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			 NULL),
		   EBUSY);

	TEST_RES(write(release[1], &token, 1), _ret == 1);

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	TEST_SUCC(close(ready[0]));
	TEST_SUCC(close(release[1]));

	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			NULL));
}
END_TEST()

FN_TEST(shared_mapping_keeps_mount_busy_after_close)
{
	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND, NULL));

	int fd = TEST_SUCC(open(ALIAS "/file", O_RDWR));
	long page_size = TEST_RES(sysconf(_SC_PAGESIZE), _ret > 0);
	void *mapping = TEST_RES(mmap(NULL, page_size, PROT_READ | PROT_WRITE,
				      MAP_SHARED, fd, 0),
				 _ret != MAP_FAILED);

	TEST_SUCC(close(fd));

	TEST_ERRNO(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			 NULL),
		   EBUSY);

	TEST_SUCC(munmap(mapping, page_size));

	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			NULL));
}
END_TEST()

FN_TEST(failed_open_does_not_leave_mount_busy)
{
	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND, NULL));
	TEST_SUCC(chmod(SOURCE "/file", 0444));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setgid(1000));
		CHECK(setuid(1000));

		CHECK_WITH(open(ALIAS "/file", O_WRONLY),
			   _ret == -1 && errno == EACCES);

		_exit(EXIT_SUCCESS);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);

	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			NULL));
	TEST_SUCC(chmod(SOURCE "/file", 0666));
}
END_TEST()

FN_TEST(readonly_and_special_opens_do_not_block_bind_remount)
{
	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND, NULL));

	int read_fd = TEST_SUCC(open(ALIAS "/file", O_RDONLY));
	int path_fd = TEST_SUCC(open(ALIAS "/file", O_PATH | O_RDWR));
	int fifo_fd = TEST_SUCC(open(ALIAS "/fifo", O_RDWR | O_NONBLOCK));
	int device_fd = TEST_SUCC(open(ALIAS "/null", O_WRONLY));

	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			NULL));

	TEST_SUCC(close(read_fd));
	TEST_SUCC(close(path_fd));
	TEST_SUCC(close(fifo_fd));
	TEST_SUCC(close(device_fd));
}
END_TEST()

FN_TEST(bind_mounts_track_writers_independently)
{
	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND, NULL));

	int source_fd = TEST_SUCC(open(SOURCE "/file", O_RDWR));
	int alias_fd = TEST_SUCC(open(ALIAS "/file", O_RDWR));

	// A newly cloned mount must start with no writers of its own.
	TEST_SUCC(mkdir(OTHER_ALIAS, 0755));
	TEST_SUCC(mount(ALIAS, OTHER_ALIAS, NULL, MS_BIND, NULL));

	TEST_SUCC(mount(NULL, OTHER_ALIAS, NULL,
			MS_REMOUNT | MS_BIND | MS_RDONLY, NULL));

	TEST_ERRNO(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			 NULL),
		   EBUSY);

	TEST_SUCC(close(alias_fd));

	// The source mount's writer must not prevent remounting the alias.
	TEST_SUCC(mount(NULL, ALIAS, NULL, MS_REMOUNT | MS_BIND | MS_RDONLY,
			NULL));

	TEST_RES(pwrite(source_fd, CONTENT, sizeof(CONTENT) - 1, 0),
		 _ret == sizeof(CONTENT) - 1);
	TEST_SUCC(close(source_fd));

	TEST_SUCC(umount(OTHER_ALIAS));
	TEST_SUCC(rmdir(OTHER_ALIAS));
}
END_TEST()

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

// Existing files on a read-only mount must not be writable,
// even when their inode permissions allow writes.
FN_TEST(readonly_rejects_writable_opens_and_truncate)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(setgid(1000));
		CHECK(setuid(1000));

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
			CHECK_WITH(open(ALIAS "/file", flags[i], 0666),
				   _ret == -1 && errno == EROFS);
		}

		CHECK_WITH(truncate(ALIAS "/file", 0),
			   _ret == -1 && errno == EROFS);
		CHECK_WITH(truncate(ALIAS "/file", sizeof(CONTENT) - 1),
			   _ret == -1 && errno == EROFS);

		int fd = CHECK(open(ALIAS "/file", O_RDONLY));

		char buf[sizeof(CONTENT)] = { 0 };
		CHECK_WITH(read(fd, buf, sizeof(buf)),
			   _ret == sizeof(CONTENT) - 1);
		CHECK_WITH(memcmp(buf, CONTENT, sizeof(CONTENT) - 1),
			   _ret == 0);

		struct stat st = { 0 };
		CHECK_WITH(fstat(fd, &st),
			   _ret >= 0 && st.st_size == sizeof(CONTENT) - 1);

		CHECK(close(fd));

		exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == EXIT_SUCCESS);
}
END_TEST()

FN_TEST(readonly_allows_path_and_special_file_opens)
{
	int fd;

	fd = TEST_SUCC(open(ALIAS "/file", O_PATH | O_RDWR | O_TRUNC));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(ALIAS "/file", O_RDONLY | O_CREAT, 0666));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(ALIAS, O_RDONLY | O_DIRECTORY));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(ALIAS "/fifo", O_RDWR | O_NONBLOCK));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(ALIAS "/null", O_WRONLY));
	TEST_RES(write(fd, "x", 1), _ret == 1);
	TEST_SUCC(close(fd));

	TEST_ERRNO(open(ALIAS, O_WRONLY), EISDIR);
}
END_TEST()

FN_TEST(readonly_rejects_xattr_changes)
{
	int fd = TEST_SUCC(open(ALIAS "/file", O_RDONLY));

	TEST_ERRNO(setxattr(ALIAS "/file", XATTR_NAME, "new", 3, 0), EROFS);
	TEST_ERRNO(lsetxattr(ALIAS "/file", XATTR_NAME, "new", 3, 0), EROFS);
	TEST_ERRNO(fsetxattr(fd, XATTR_NAME, "new", 3, 0), EROFS);
	TEST_ERRNO(removexattr(ALIAS "/file", XATTR_NAME), EROFS);
	TEST_ERRNO(lremovexattr(ALIAS "/file", XATTR_NAME), EROFS);
	TEST_ERRNO(fremovexattr(fd, XATTR_NAME), EROFS);

	char value[sizeof(XATTR_VALUE)] = { 0 };
	TEST_RES(getxattr(ALIAS "/file", XATTR_NAME, value, sizeof(value)),
		 _ret == sizeof(XATTR_VALUE) - 1);
	TEST_RES(memcmp(value, XATTR_VALUE, sizeof(XATTR_VALUE) - 1),
		 _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(remove_fixtures)
{
	CHECK(umount(ALIAS));

	CHECK(unlink(SOURCE "/file"));
	CHECK(unlink(SOURCE "/fifo"));
	CHECK(unlink(SOURCE "/null"));
	CHECK(umount(SOURCE));

	CHECK(rmdir(ALIAS));
	CHECK(rmdir(SOURCE));
}
END_SETUP()
