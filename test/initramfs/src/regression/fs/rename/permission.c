// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/tmp/rename_directory_dac"
#define OLD_PARENT BASE_DIR "/old"
#define NEW_PARENT BASE_DIR "/new"
#define SOURCE_DIR OLD_PARENT "/source"
#define TARGET_DIR NEW_PARENT "/target"
#define MOVED_DIR NEW_PARENT "/source"
#define NON_OWNER_UID 1000

static void remove_if_exists(const char *path)
{
	CHECK_WITH(rmdir(path), _ret == 0 || errno == ENOENT);
}

static void cleanup_test_tree(void)
{
	remove_if_exists(MOVED_DIR);
	remove_if_exists(SOURCE_DIR);
	remove_if_exists(TARGET_DIR);
	remove_if_exists(OLD_PARENT);
	remove_if_exists(NEW_PARENT);
	remove_if_exists(BASE_DIR);
}

static void create_writable_parents(void)
{
	cleanup_test_tree();
	CHECK(mkdir(BASE_DIR, 0777));
	CHECK(mkdir(OLD_PARENT, 0777));
	CHECK(mkdir(NEW_PARENT, 0777));
	CHECK(chmod(BASE_DIR, 0777));
	CHECK(chmod(OLD_PARENT, 0777));
	CHECK(chmod(NEW_PARENT, 0777));
}

static void become_non_owner(void)
{
	CHECK(setresgid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
	CHECK(setresuid(NON_OWNER_UID, NON_OWNER_UID, NON_OWNER_UID));
}

static void wait_for_successful_child(pid_t pid)
{
	int status;

	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

FN_SETUP(cleanup_before_tests)
{
	cleanup_test_tree();
}
END_SETUP()

FN_TEST(cross_parent_directory_rename_requires_source_write_permission)
{
	pid_t pid;

	create_writable_parents();
	TEST_SUCC(mkdir(SOURCE_DIR, 0755));
	TEST_SUCC(chmod(SOURCE_DIR, 0555));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		CHECK_WITH(rename(SOURCE_DIR, MOVED_DIR),
			   _ret == -1 && errno == EACCES);
		_exit(0);
	}
	wait_for_successful_child(pid);
	TEST_SUCC(access(SOURCE_DIR, F_OK));
	TEST_ERRNO(access(MOVED_DIR, F_OK), ENOENT);
	cleanup_test_tree();
}
END_TEST()

FN_TEST(cross_parent_directory_rename_allows_writable_source)
{
	pid_t pid;

	create_writable_parents();
	TEST_SUCC(mkdir(SOURCE_DIR, 0755));
	TEST_SUCC(chmod(SOURCE_DIR, 0777));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		CHECK(rename(SOURCE_DIR, MOVED_DIR));
		_exit(0);
	}
	wait_for_successful_child(pid);
	TEST_ERRNO(access(SOURCE_DIR, F_OK), ENOENT);
	TEST_SUCC(access(MOVED_DIR, F_OK));
	cleanup_test_tree();
}
END_TEST()

FN_TEST(rename_exchange_requires_destination_directory_write_permission)
{
	pid_t pid;

	create_writable_parents();
	TEST_SUCC(mkdir(SOURCE_DIR, 0755));
	TEST_SUCC(mkdir(TARGET_DIR, 0755));
	TEST_SUCC(chmod(SOURCE_DIR, 0777));
	TEST_SUCC(chmod(TARGET_DIR, 0555));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		become_non_owner();
		CHECK_WITH(syscall(SYS_renameat2, AT_FDCWD, SOURCE_DIR, AT_FDCWD,
				   TARGET_DIR, RENAME_EXCHANGE),
			   _ret == -1 && errno == EACCES);
		_exit(0);
	}
	wait_for_successful_child(pid);
	TEST_SUCC(access(SOURCE_DIR, F_OK));
	TEST_SUCC(access(TARGET_DIR, F_OK));
	cleanup_test_tree();
}
END_TEST()
