// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <fcntl.h>
#include <sched.h>
#include <sys/mount.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <sys/syscall.h>

#include "../../common/test.h"

#define STACK_SIZE (1024 * 1024)

// --- Test for unshare(CLONE_NEWNS) ---

#define UNSHARE_MNT "/mnt/unshare_test"
#define UNSHARE_FILE UNSHARE_MNT "/child.txt"

static int unshare_child_fn(void)
{
	CHECK(unshare(CLONE_NEWNS));

	// Mount a tmpfs in the new namespace. This should not be visible to the parent.
	CHECK(mount("ramfs_child", UNSHARE_MNT, "ramfs", 0, ""));

	int fd = CHECK(open(UNSHARE_FILE, O_CREAT | O_WRONLY, 0644));
	CHECK(close(fd));
	CHECK(access(UNSHARE_FILE, F_OK));

	CHECK(umount(UNSHARE_MNT));

	CHECK_WITH(access(UNSHARE_FILE, F_OK), _ret == -1 && errno == ENOENT);

	return 0;
}

FN_TEST(unshare_newns)
{
	// Setup
	CHECK_WITH(mkdir("/mnt", 0755), _ret >= 0 || errno == EEXIST);
	CHECK_WITH(mkdir(UNSHARE_MNT, 0755), _ret >= 0 || errno == EEXIST);

	TEST_ERRNO(access(UNSHARE_FILE, F_OK), ENOENT);

	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		exit(unshare_child_fn());
	} else {
		int status;
		TEST_SUCC(waitpid(pid, &status, 0));
		TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);
		// Verify that the child's mount operations were not visible to the parent.
		TEST_ERRNO(access(UNSHARE_FILE, F_OK), ENOENT);
	}

	// Teardown
	TEST_SUCC(rmdir(UNSHARE_MNT));
}
END_TEST()

// --- Test for clone(CLONE_NEWNS) ---

#define CLONE_PARENT_MNT "/mnt/clone_parent"
#define CLONE_CHILD_MNT "/mnt/clone_child"
#define PARENT_FILE CLONE_PARENT_MNT "/parent.txt"
#define CHILD_FILE CLONE_CHILD_MNT "/child.txt"

static int clone_child_fn(void *arg)
{
	CHECK(access(PARENT_FILE, F_OK));

	CHECK(mount("ramfs_child", CLONE_CHILD_MNT, "ramfs", 0, ""));
	int fd = CHECK(open(CHILD_FILE, O_CREAT | O_WRONLY, 0644));
	CHECK(close(fd));

	CHECK(umount(CLONE_PARENT_MNT));

	// Verify parent's mount is gone from child's view.
	CHECK_WITH(access(PARENT_FILE, F_OK), errno == ENOENT);

	// Verify child's own mount is still present.
	CHECK(access(CHILD_FILE, F_OK));

	return 0;
}

FN_TEST(clone_newns)
{
	// Setup
	CHECK_WITH(mkdir("/mnt", 0755), _ret >= 0 || errno == EEXIST);
	CHECK_WITH(mkdir(CLONE_PARENT_MNT, 0755), _ret >= 0 || errno == EEXIST);
	CHECK_WITH(mkdir(CLONE_CHILD_MNT, 0755), _ret >= 0 || errno == EEXIST);

	TEST_SUCC(mount("ramfs_parent", CLONE_PARENT_MNT, "ramfs", 0, ""));
	int fd = TEST_SUCC(open(PARENT_FILE, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(fd));

	char *stack = malloc(STACK_SIZE);
	char *stack_top = stack + STACK_SIZE;

	pid_t pid = TEST_SUCC(
		clone(clone_child_fn, stack_top, CLONE_NEWNS | SIGCHLD, NULL));

	int status;
	TEST_SUCC(waitpid(pid, &status, 0));
	TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

	// Parent's mount should be unaffected by child's umount.
	TEST_SUCC(access(PARENT_FILE, F_OK));
	// Child's mount should not be visible to the parent.
	TEST_ERRNO(access(CHILD_FILE, F_OK), ENOENT);

	// Teardown
	free(stack);
	TEST_SUCC(umount(CLONE_PARENT_MNT));
	TEST_SUCC(rmdir(CLONE_PARENT_MNT));
	TEST_SUCC(rmdir(CLONE_CHILD_MNT));
}
END_TEST()

// --- Test for setns(CLONE_NEWNS) ---

#define SETNS_MNT "/mnt/setns_target"
#define SETNS_FILE SETNS_MNT "/target.txt"

// This function runs in a child process to set up a target namespace.
static int setns_target_fn(int pipe_write_fd)
{
	CHECK(unshare(CLONE_NEWNS));

	CHECK(mount("ramfs_target", SETNS_MNT, "ramfs", 0, ""));

	int fd = CHECK(open(SETNS_FILE, O_CREAT | O_WRONLY, 0644));
	CHECK(close(fd));

	// Signal to the parent that setup is complete.
	char ok = 'K';
	CHECK(write(pipe_write_fd, &ok, 1));
	CHECK(close(pipe_write_fd));

	// Wait to be killed by the parent.
	pause();
	return 0;
}

FN_TEST(setns_newns)
{
	// Setup
	CHECK_WITH(mkdir("/mnt", 0755), _ret >= 0 || errno == EEXIST);
	CHECK_WITH(mkdir(SETNS_MNT, 0755), _ret >= 0 || errno == EEXIST);

	int pipefd[2];
	TEST_SUCC(pipe(pipefd));

	pid_t child_pid = TEST_SUCC(fork());

	if (child_pid == 0) {
		close(pipefd[0]);
		exit(setns_target_fn(pipefd[1]));
	}

	close(pipefd[1]);

	char buf[10];
	TEST_SUCC(read(pipefd[0], &buf, 1));
	close(pipefd[0]);

	int pid_fd = TEST_SUCC(syscall(SYS_pidfd_open, child_pid, 0));

	// Switching into the child's UTS namespace will not change the CWD.
	TEST_SUCC(chdir("/mnt"));
	TEST_SUCC(setns(pid_fd, CLONE_NEWUTS));
	TEST_RES(getcwd(buf, sizeof(buf)), strcmp(buf, "/mnt") == 0);

	// Switching into the child's mount namespace will reset the CWD.
	TEST_SUCC(setns(pid_fd, CLONE_NEWNS));
	TEST_RES(getcwd(buf, sizeof(buf)), strcmp(buf, "/") == 0);

	TEST_SUCC(close(pid_fd));

	// Check if we can see the file created by the child in its namespace.
	TEST_SUCC(access(SETNS_FILE, F_OK));

	// Teardown
	TEST_SUCC(kill(child_pid, SIGKILL));
	TEST_SUCC(waitpid(child_pid, NULL, 0));

	TEST_SUCC(umount(SETNS_MNT));
	TEST_SUCC(rmdir(SETNS_MNT));
}
END_TEST()

#define COMPLEX_BASE "/test0"
#define COMPLEX_CONTENT "/test0/content"
#define COMPLEX_FILE "/test0/content/hello.txt"

FN_TEST(complex_mount_tree_unshare)
{
	// Setup: Create directory structure
	CHECK_WITH(mkdir(COMPLEX_BASE, 0755), _ret >= 0 || errno == EEXIST);

	// Mount tmpfs on test0 (first layer)
	TEST_SUCC(mount("none", COMPLEX_BASE, "tmpfs", 0, ""));

	// Mount tmpfs on test0 again (second layer, stacked mount)
	TEST_SUCC(mount("none", COMPLEX_BASE, "tmpfs", 0, ""));

	// Recreate content directory on the new mount
	TEST_SUCC(mkdir(COMPLEX_CONTENT, 0755));

	// Mount tmpfs on test0/content (third layer)
	TEST_SUCC(mount("none", COMPLEX_CONTENT, "tmpfs", 0, ""));

	// Create file with content
	int fd = TEST_SUCC(open(COMPLEX_FILE, O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(write(fd, "hello world\n", 12));
	TEST_SUCC(close(fd));

	// Verify file exists in parent
	TEST_SUCC(access(COMPLEX_FILE, F_OK));

	// Fork and unshare in child
	pid_t pid = TEST_SUCC(fork());

	if (pid == 0) {
		TEST_SUCC(unshare(CLONE_NEWNS));

		// Try to read the file created in the parent's mount tree
		int fd = CHECK(open(COMPLEX_FILE, O_RDONLY));

		char buf[32] = { 0 };
		TEST_SUCC(read(fd, buf, sizeof(buf) - 1));
		TEST_SUCC(close(fd));

		// Verify the content
		CHECK_WITH(strcmp(buf, "hello world\n"), _ret == 0);

		// Verify we can still access the nested mount structure
		TEST_SUCC(access(COMPLEX_CONTENT, F_OK));
		TEST_SUCC(access(COMPLEX_BASE, F_OK));

		exit(0);
	} else {
		int status;
		TEST_SUCC(waitpid(pid, &status, 0));
		TEST_RES(WIFEXITED(status) && WEXITSTATUS(status), _ret == 0);

		// Verify parent's mount tree is unchanged
		TEST_SUCC(access(COMPLEX_FILE, F_OK));

		// Teardown: Unmount in reverse order
		TEST_SUCC(umount(COMPLEX_CONTENT));
		TEST_SUCC(rmdir(COMPLEX_CONTENT));
		TEST_SUCC(umount(COMPLEX_BASE));
		TEST_SUCC(umount(COMPLEX_BASE));
		TEST_SUCC(rmdir(COMPLEX_BASE));
	}
}
END_TEST()