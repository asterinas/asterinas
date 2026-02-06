// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <dirent.h>
#include <fcntl.h>
#include <limits.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

FN_SETUP(create_chroot_env)
{
	// Create chroot target structure
	CHECK_WITH(mkdir("/foo", 0755), _ret >= 0 || errno == EEXIST);
	CHECK_WITH(mkdir("/foo/proc", 0755), _ret >= 0 || errno == EEXIST);
	CHECK_WITH(mkdir("/foo/nix", 0755), _ret >= 0 || errno == EEXIST);

	// Perform bind mounts
	CHECK(mount("proc", "/foo/proc", "proc", 0, ""));
	CHECK(mount("/nix", "/foo/nix", NULL, MS_BIND, NULL));
}
END_SETUP()

// Helper function to check if a directory does NOT contain a specific entry
static int dir_not_contains(const char *path, const char *entry_name)
{
	DIR *dir = CHECK(opendir(path));
	struct dirent *entry;
	while ((entry = readdir(dir)) != NULL) {
		if (strcmp(entry->d_name, entry_name) == 0) {
			CHECK(closedir(dir));
			return -1; // Found the entry, which means failure
		}
	}
	CHECK(closedir(dir));
	return 0; // Entry not found, success
}

// Helper function to read a file and check for a substring
static int file_contains(const char *filepath, const char *substring)
{
	int fd = CHECK(open(filepath, O_RDONLY));
	char buf[4096] = { 0 };
	CHECK(read(fd, buf, sizeof(buf) - 1));
	CHECK(close(fd));
	return strstr(buf, substring) != NULL ? 0 : -1;
}

// Macro to wait for a child process and check its exit status
#define WAIT_AND_CHECK_CHILD(pid)                            \
	do {                                                 \
		int status;                                  \
		TEST_RES(waitpid(pid, &status, 0),           \
			 _ret == pid && WIFEXITED(status) && \
				 WEXITSTATUS(status) == 0);  \
	} while (0)

FN_TEST(chroot_getcwd)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(chroot("/foo"));
		CHECK(chdir("/"));

		char cwd[PATH_MAX];
		CHECK_WITH(getcwd(cwd, sizeof(cwd)), strcmp(cwd, "/") == 0);

		exit(EXIT_SUCCESS);
	} else {
		WAIT_AND_CHECK_CHILD(pid);
	}
}
END_TEST()

FN_TEST(chroot_chdir)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(chroot("/foo"));
		CHECK(chdir("/"));
		CHECK(chdir(".."));

		// Verify we can't see 'foo' directory
		CHECK(dir_not_contains(".", "foo"));

		// Verify we are still at root
		char cwd[PATH_MAX];
		CHECK_WITH(getcwd(cwd, sizeof(cwd)), strcmp(cwd, "/") == 0);

		exit(EXIT_SUCCESS);
	} else {
		WAIT_AND_CHECK_CHILD(pid);
	}
}
END_TEST()

FN_TEST(chroot_mountinfo)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(chroot("/foo"));
		CHECK(chdir("/"));

		CHECK(file_contains("/proc/self/mountinfo", "/nix /nix"));

		exit(EXIT_SUCCESS);
	} else {
		WAIT_AND_CHECK_CHILD(pid);
	}
}
END_TEST()

FN_TEST(chroot_fd_escape)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		int pre_chroot_fd =
			CHECK(open("/test", O_RDONLY | O_DIRECTORY));

		CHECK(chroot("/foo"));
		CHECK(chdir("/"));

		// Use fchdir to go to the pre-chroot /test directory
		CHECK(syscall(SYS_fchdir, pre_chroot_fd));

		// Now getcwd should add "(unreachable)" prefix because we're outside the chroot jail
		char cwd[PATH_MAX];
		CHECK_WITH(syscall(SYS_getcwd, cwd, sizeof(cwd)),
			   strcmp(cwd, "(unreachable)/test") == 0);

		// But we should be able to see 'foo' directory by listing parent
		CHECK(chdir(".."));
		CHECK_WITH(dir_not_contains(".", "foo"), _ret == -1);

		CHECK(close(pre_chroot_fd));
		exit(EXIT_SUCCESS);
	} else {
		WAIT_AND_CHECK_CHILD(pid);
	}
}
END_TEST()

FN_SETUP(cleanup_chroot_env)
{
	CHECK(umount("/foo/proc"));
	CHECK(umount("/foo/nix"));
	CHECK(rmdir("/foo/proc"));
	CHECK(rmdir("/foo/nix"));
	CHECK(rmdir("/foo"));
}
END_SETUP()