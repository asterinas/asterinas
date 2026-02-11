// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <dirent.h>
#include <fcntl.h>
#include <errno.h>
#include <limits.h>
#include <linux/nsfs.h>
#include <linux/sched.h>
#include <poll.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define NS_DIR "/proc/self/ns"

/*
 * Supported namespace entries.
 *
 * `ns_files` lists the filenames under /proc/[pid]/ns/.
 * `ns_names` lists the names as they appear in readlink(2) output.
 *   - For most namespaces these are identical to `ns_files`.
 *   - For "pid_for_children" and "time_for_children", readlink(2) shows
 *     "pid" and "time" respectively (not applicable to the entries below,
 *     but noted here for future reference).
 * `clone_flags` lists the corresponding CLONE_NEW* flag for each entry.
 */
static const char *ns_files[] = { "uts", "mnt", "user" };
static const char *ns_names[] = { "uts", "mnt", "user" };
static const int clone_flags[] = { CLONE_NEWUTS, CLONE_NEWNS, CLONE_NEWUSER };
static const size_t ns_count = sizeof(ns_files) / sizeof(ns_files[0]);

/* -------------------------------------------------------------------------- */

/*
 * Verify basic filesystem semantics of namespace files:
 * access modes, read/write behaviour, poll events, stat, and seek.
 */
FN_TEST(common_fs_operations)
{
	char path[PATH_MAX];
	char buf[1];

	for (size_t i = 0; i < ns_count; i++) {
		snprintf(path, sizeof(path), "%s/%s", NS_DIR, ns_files[i]);

		/* Namespace files must not be opened for writing. */
		TEST_ERRNO(open(path, O_RDWR), EPERM);
		TEST_ERRNO(open(path, O_WRONLY), EPERM);

		int nsfd = TEST_SUCC(open(path, O_RDONLY));

		/* read(2) and write(2) are not supported. */
		TEST_ERRNO(read(nsfd, buf, 1), EINVAL);
		TEST_ERRNO(write(nsfd, buf, 1), EBADF);

		/* poll(2) should report IN, OUT, and RDNORM immediately. */
		struct pollfd pfd = {
			.fd = nsfd,
			.events = POLLIN | POLLOUT | POLLRDHUP | POLLPRI |
				  POLLRDNORM,
		};
		TEST_RES(poll(&pfd, 1, -1),
			 pfd.revents == (POLLIN | POLLOUT | POLLRDNORM));

		/* The file should appear as a regular file with mode 0444. */
		struct stat64 st;
		TEST_RES(fstat64(nsfd, &st), st.st_mode == (S_IFREG | 0444));

		/* Seeking is not supported. */
		TEST_ERRNO(lseek64(nsfd, 0, SEEK_SET), ESPIPE);

		TEST_SUCC(close(nsfd));
	}
}
END_TEST()

/* -------------------------------------------------------------------------- */

/*
 * Verify that readlink(2) on a namespace symlink returns "<type>:[<ino>]".
 */
FN_TEST(readlink)
{
	char path[PATH_MAX];
	char link[256];

	for (size_t i = 0; i < ns_count; i++) {
		snprintf(path, sizeof(path), "%s/%s", NS_DIR, ns_files[i]);

		int nsfd = TEST_SUCC(open(path, O_RDONLY));

		struct stat st;
		TEST_SUCC(fstat(nsfd, &st));
		TEST_SUCC(close(nsfd));

		char expected[256];
		snprintf(expected, sizeof(expected), "%s:[%lu]", ns_names[i],
			 st.st_ino);

		memset(link, 0, sizeof(link));
		TEST_RES(readlink(path, link, sizeof(link) - 1),
			 strcmp(expected, link) == 0);
	}
}
END_TEST()

/* -------------------------------------------------------------------------- */

/*
 * Verify namespace file accessibility for a zombie process.
 *
 * After a child exits but before it is fully reaped, its "pid" and "user"
 * namespace files should still be accessible, while others (e.g. "uts",
 * "mnt") should return ENOENT.
 */
FN_TEST(zombie_process)
{
	char path[PATH_MAX];

	pid_t pid = fork();
	TEST_RES(pid >= 0, 1);

	if (pid == 0) {
		/* Child exits immediately to become a zombie. */
		exit(0);
	}

	/* Wait without reaping so the child remains a zombie. */
	TEST_SUCC(waitid(P_PID, pid, NULL, WNOWAIT | WEXITED));

	for (size_t i = 0; i < ns_count; i++) {
		snprintf(path, sizeof(path), "/proc/%d/ns/%s", pid,
			 ns_files[i]);

		/*
		 * "pid" and "user" namespaces are still reachable for a zombie;
		 * all other namespace files should have disappeared.
		 */
		if (strcmp(ns_files[i], "pid") == 0 ||
		    strcmp(ns_files[i], "user") == 0) {
			char link[256] = { 0 };
			TEST_SUCC(readlink(path, link, sizeof(link) - 1));

			int nsfd = TEST_SUCC(open(path, O_RDONLY));
			TEST_SUCC(close(nsfd));
		} else {
			TEST_ERRNO(open(path, O_RDONLY), ENOENT);
		}
	}

	/* Fully reap the child. */
	TEST_SUCC(waitpid(pid, NULL, 0));
}
END_TEST()

/* -------------------------------------------------------------------------- */

/*
 * Exercise the NS_GET_* ioctl commands on every namespace type.
 */
FN_TEST(ioctl)
{
	char path[PATH_MAX];

	for (size_t i = 0; i < ns_count; i++) {
		snprintf(path, sizeof(path), "%s/%s", NS_DIR, ns_files[i]);
		int nsfd = TEST_SUCC(open(path, O_RDONLY));
		int is_user_ns = (strcmp(ns_files[i], "user") == 0);

		/*
		 * NS_GET_USERNS: returns the owning user namespace fd.
		 * For a user namespace itself this is not permitted.
		 */
		if (!is_user_ns) {
			int userns_fd = TEST_SUCC(ioctl(nsfd, NS_GET_USERNS));
			TEST_SUCC(close(userns_fd));
		} else {
			TEST_ERRNO(ioctl(nsfd, NS_GET_USERNS), EPERM);
		}

		/*
		 * NS_GET_PARENT: returns the parent namespace fd.
		 * Non-hierarchical namespaces return EINVAL;
		 * the user namespace returns EPERM.
		 */
		if (!is_user_ns) {
			TEST_ERRNO(ioctl(nsfd, NS_GET_PARENT), EINVAL);
		} else {
			TEST_ERRNO(ioctl(nsfd, NS_GET_PARENT), EPERM);
		}

		/* NS_GET_NSTYPE: should match the corresponding clone flag. */
		TEST_RES(ioctl(nsfd, NS_GET_NSTYPE), _ret == clone_flags[i]);

		/*
		 * NS_GET_OWNER_UID: only valid for user namespaces.
		 * A NULL pointer should yield EFAULT.
		 */
		if (!is_user_ns) {
			TEST_ERRNO(ioctl(nsfd, NS_GET_OWNER_UID), EINVAL);
		} else {
			TEST_ERRNO(ioctl(nsfd, NS_GET_OWNER_UID, 0), EFAULT);

			uid_t uid;
			TEST_SUCC(ioctl(nsfd, NS_GET_OWNER_UID, &uid));
		}

		TEST_SUCC(close(nsfd));
	}
}
END_TEST()

/* -------------------------------------------------------------------------- */

/*
 * Test setns(2) with the current process's own namespaces, an invalid fd,
 * and across a fork boundary.
 */
FN_TEST(setns)
{
	char path[PATH_MAX];

	/* Joining our own namespace should succeed (except for user ns). */
	for (size_t i = 0; i < ns_count; i++) {
		snprintf(path, sizeof(path), "%s/%s", NS_DIR, ns_files[i]);
		int nsfd = TEST_SUCC(open(path, O_RDONLY));

		if (strcmp(ns_files[i], "user") != 0) {
			TEST_SUCC(setns(nsfd, 0));
		} else {
			TEST_ERRNO(setns(nsfd, 0), EINVAL);
		}

		TEST_SUCC(close(nsfd));
	}

	/* An invalid fd must fail with EBADF. */
	TEST_ERRNO(setns(-1, 0), EBADF);

	/* A child process should be able to join its parent's UTS namespace. */
	pid_t pid = fork();
	TEST_RES(pid >= 0, 1);

	if (pid == 0) {
		snprintf(path, sizeof(path), "/proc/%d/ns/uts", getppid());
		int parent_ns = CHECK(open(path, O_RDONLY));
		CHECK(setns(parent_ns, 0));
		close(parent_ns);
		exit(0);
	}

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

/* -------------------------------------------------------------------------- */

/*
 * Verify that /proc/self/fd/<nsfd> resolves to the same target as the
 * original /proc/self/ns/<type> symlink.
 */
FN_TEST(proc_fd_name)
{
	char path[PATH_MAX];
	char link_ns[256];
	char link_fd[256];

	for (size_t i = 0; i < ns_count; i++) {
		snprintf(path, sizeof(path), "%s/%s", NS_DIR, ns_files[i]);

		memset(link_ns, 0, sizeof(link_ns));
		TEST_SUCC(readlink(path, link_ns, sizeof(link_ns) - 1));

		int nsfd = TEST_SUCC(open(path, O_RDONLY));

		snprintf(path, sizeof(path), "/proc/self/fd/%d", nsfd);

		memset(link_fd, 0, sizeof(link_fd));
		TEST_RES(readlink(path, link_fd, sizeof(link_fd) - 1),
			 strcmp(link_ns, link_fd) == 0);

		TEST_SUCC(close(nsfd));
	}
}
END_TEST()

/* -------------------------------------------------------------------------- */

static pid_t sys_clone3(struct clone_args *args)
{
	return syscall(SYS_clone3, args, sizeof(struct clone_args));
}

/*
 * Verify that a namespace outlives its creator as long as an open fd exists.
 *
 * A child is created in a new UTS namespace and then exits. The parent,
 * which still holds an nsfd obtained before the child exited, must be able
 * to query and join that namespace even after the child is reaped.
 */
FN_TEST(lifetime)
{
	struct clone_args args = {
		.flags = CLONE_NEWUTS,
		.exit_signal = SIGCHLD,
	};
	pid_t pid = TEST_SUCC(sys_clone3(&args));

	if (pid == 0) {
		sleep(1);
		exit(0);
	}

	/* Open the child's UTS namespace while it is still alive. */
	char path[PATH_MAX];
	snprintf(path, sizeof(path), "/proc/%d/ns/uts", pid);
	int nsfd = TEST_SUCC(open(path, O_RDONLY));

	TEST_RES(ioctl(nsfd, NS_GET_NSTYPE), _ret == CLONE_NEWUTS);
	int user_ns_fd = TEST_SUCC(ioctl(nsfd, NS_GET_USERNS));
	TEST_SUCC(close(user_ns_fd));

	/* Reap the child. */
	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	/*
	 * The namespace must still be usable via the held fd even though the
	 * child process no longer exists.
	 */
	TEST_RES(ioctl(nsfd, NS_GET_NSTYPE), _ret == CLONE_NEWUTS);
	user_ns_fd = TEST_SUCC(ioctl(nsfd, NS_GET_USERNS));
	TEST_SUCC(close(user_ns_fd));
	TEST_SUCC(setns(nsfd, 0));

	TEST_SUCC(close(nsfd));
}
END_TEST()
