// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define CGROUP_ROOT "/sys/fs/cgroup"
#define TEST_BASE_CGROUP CGROUP_ROOT "/cgns-base"
#define TEST_LEAF_CGROUP TEST_BASE_CGROUP "/leaf"
#define TEST_SIBLING_CGROUP CGROUP_ROOT "/cgns-sibling"
#define TEST_MOUNT_DIR "/tmp/cgns-mnt"
#define TEST_MOUNT_LEAF TEST_MOUNT_DIR "/leaf"
#define TEST_MOUNT_SIBLING TEST_MOUNT_DIR "/cgns-sibling"

static int g_initial_cgroup_ns_fd;
static int g_initial_mount_ns_fd;

static int move_pid_to_cgroup(const char *dir, pid_t pid)
{
	char path[PATH_MAX];
	char pid_text[32];
	int fd;
	ssize_t count;

	snprintf(path, sizeof(path), "%s/cgroup.procs", dir);
	fd = open(path, O_WRONLY);
	if (fd < 0)
		return -1;

	snprintf(pid_text, sizeof(pid_text), "%d", pid);
	count = write(fd, pid_text, strlen(pid_text));
	close(fd);
	if (count != (ssize_t)strlen(pid_text)) {
		if (count >= 0)
			errno = EIO;
		return -1;
	}

	return 0;
}

static int check_proc_cgroup(pid_t pid, const char *expected)
{
	char path[64];
	char content[PATH_MAX];
	int fd;
	ssize_t count;

	snprintf(path, sizeof(path), "/proc/%d/cgroup", pid);
	fd = open(path, O_RDONLY);
	if (fd < 0)
		return -1;

	count = read(fd, content, sizeof(content) - 1);
	close(fd);
	if (count < 0)
		return -1;

	content[count] = '\0';
	if (strcmp(content, expected) != 0) {
		errno = EIO;
		return -1;
	}

	return 0;
}

static int read_namespace_inode(const char *path, ino_t *inode)
{
	struct stat stat_buf;

	if (stat(path, &stat_buf) < 0)
		return -1;

	*inode = stat_buf.st_ino;
	return 0;
}

static int read_self_cgroup_ns_inode(ino_t *inode)
{
	return read_namespace_inode("/proc/self/ns/cgroup", inode);
}

static int read_task_cgroup_ns_inode(pid_t tid, ino_t *inode)
{
	char path[64];

	snprintf(path, sizeof(path), "/proc/self/task/%d/ns/cgroup", tid);
	return read_namespace_inode(path, inode);
}

static int read_process_cgroup_ns_inode(pid_t pid, ino_t *inode)
{
	char path[64];

	snprintf(path, sizeof(path), "/proc/%d/ns/cgroup", pid);
	return read_namespace_inode(path, inode);
}

static int check_mount_root(const char *mount_point, const char *expected_root)
{
	FILE *mountinfo = fopen("/proc/self/mountinfo", "r");
	char line[512];

	if (!mountinfo)
		return -1;

	while (fgets(line, sizeof(line), mountinfo)) {
		char root[256];
		char seen_mount_point[256];
		char fs_type[64];

		if (sscanf(line, "%*s %*s %*s %255s %255s %*s - %63s", root,
			   seen_mount_point, fs_type) != 3)
			continue;
		if (strcmp(seen_mount_point, mount_point) != 0)
			continue;
		if (strcmp(fs_type, "cgroup2") != 0)
			continue;

		fclose(mountinfo);
		if (strcmp(root, expected_root) != 0) {
			errno = EIO;
			return -1;
		}

		return 0;
	}

	fclose(mountinfo);
	errno = ENOENT;
	return -1;
}

static int reset_self(void)
{
	if (setns(g_initial_cgroup_ns_fd, CLONE_NEWCGROUP) < 0)
		return -1;
	if (move_pid_to_cgroup(CGROUP_ROOT, getpid()) < 0)
		return -1;
	if (setns(g_initial_mount_ns_fd, CLONE_NEWNS) < 0)
		return -1;

	return 0;
}

enum new_cgroup_ns_t {
	KEEP_OLD_CGROUP_NS = 0,
	MOVE_TO_NEW_CGROUP_NS = 1,
};

static pid_t spawn_paused_process(const char *cgroup_dir,
				  enum new_cgroup_ns_t new_cgroup)
{
	pid_t child = CHECK(fork());

	if (child == 0) {
		CHECK(move_pid_to_cgroup(cgroup_dir, getpid()));
		if (new_cgroup)
			CHECK(unshare(CLONE_NEWCGROUP));
		pause();
		_exit(0);
	}

	/* Wait until the child executes `pause()`. */
	CHECK(usleep(50000));

	return child;
}

static void kill_paused_process(pid_t pid)
{
	CHECK(kill(pid, SIGKILL));
	CHECK_WITH(waitpid(pid, NULL, 0), _ret == pid);
}

struct thread_probe {
	pthread_barrier_t ready;
	pthread_barrier_t release;
	pid_t tid;
};

static void *unshare_cgroup_ns_thread(void *arg)
{
	struct thread_probe *probe = arg;

	probe->tid = CHECK(syscall(SYS_gettid));
	CHECK(unshare(CLONE_NEWCGROUP));
	CHECK_WITH(pthread_barrier_wait(&probe->ready),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
	CHECK_WITH(pthread_barrier_wait(&probe->release),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);

	return NULL;
}

FN_SETUP(init)
{
	g_initial_cgroup_ns_fd = CHECK(open("/proc/self/ns/cgroup", O_RDONLY));
	g_initial_mount_ns_fd = CHECK(open("/proc/self/ns/mnt", O_RDONLY));

	CHECK(mkdir(TEST_BASE_CGROUP, 0755));
	CHECK(mkdir(TEST_LEAF_CGROUP, 0755));
	CHECK(mkdir(TEST_SIBLING_CGROUP, 0755));
	CHECK(mkdir(TEST_MOUNT_DIR, 0755));
}
END_SETUP()

/*
 * `unshare(CLONE_NEWCGROUP)` only changes the calling thread's namespace.
 * Other threads in the same process keep the original namespace inode.
 */
FN_TEST(cgroup_namespace_stays_thread_local)
{
	struct thread_probe probe = {};
	pthread_t thread;
	ino_t main_before;
	ino_t main_after;
	ino_t worker_ns;

	TEST_SUCC(read_self_cgroup_ns_inode(&main_before));

	TEST_RES(pthread_barrier_init(&probe.ready, NULL, 2), _ret == 0);
	TEST_RES(pthread_barrier_init(&probe.release, NULL, 2), _ret == 0);
	TEST_RES(pthread_create(&thread, NULL, unshare_cgroup_ns_thread,
				&probe),
		 _ret == 0);

	TEST_RES(pthread_barrier_wait(&probe.ready),
		 _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
	TEST_RES(read_self_cgroup_ns_inode(&main_after),
		 main_before == main_after);
	TEST_RES(read_task_cgroup_ns_inode(probe.tid, &worker_ns),
		 worker_ns != main_after);
	TEST_RES(pthread_barrier_wait(&probe.release),
		 _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);

	TEST_RES(pthread_join(thread, NULL), _ret == 0);
	TEST_RES(pthread_barrier_destroy(&probe.ready), _ret == 0);
	TEST_RES(pthread_barrier_destroy(&probe.release), _ret == 0);
}
END_TEST()

/*
 * `/proc/<pid>/cgroup` is rendered from the caller's active cgroup namespace:
 * the namespace root becomes `/`, descendants stay reachable beneath it, and
 * cgroups outside the root are shown through `..`.
 */
FN_TEST(proc_cgroup_follows_callers_namespace)
{
	int rooted_ns_fd;
	pid_t sibling_child;

	sibling_child =
		spawn_paused_process(TEST_SIBLING_CGROUP, KEEP_OLD_CGROUP_NS);
	TEST_SUCC(move_pid_to_cgroup(TEST_BASE_CGROUP, getpid()));
	TEST_SUCC(check_proc_cgroup(sibling_child, "0::/cgns-sibling\n"));
	TEST_SUCC(check_proc_cgroup(getpid(), "0::/cgns-base\n"));

	TEST_SUCC(unshare(CLONE_NEWCGROUP));
	rooted_ns_fd = TEST_SUCC(open("/proc/self/ns/cgroup", O_RDONLY));
	TEST_SUCC(check_proc_cgroup(sibling_child, "0::/../cgns-sibling\n"));
	TEST_SUCC(check_proc_cgroup(getpid(), "0::/\n"));

	TEST_SUCC(move_pid_to_cgroup(TEST_LEAF_CGROUP, getpid()));
	TEST_SUCC(check_proc_cgroup(getpid(), "0::/leaf\n"));

	TEST_SUCC(setns(g_initial_cgroup_ns_fd, CLONE_NEWCGROUP));
	TEST_SUCC(move_pid_to_cgroup(CGROUP_ROOT, getpid()));
	TEST_SUCC(check_proc_cgroup(getpid(), "0::/\n"));

	TEST_SUCC(setns(rooted_ns_fd, CLONE_NEWCGROUP));
	TEST_SUCC(check_proc_cgroup(getpid(), "0::/..\n"));

	TEST_SUCC(reset_self());
	TEST_SUCC(close(rooted_ns_fd));
	kill_paused_process(sibling_child);
}
END_TEST()

/*
 * Joining another task's cgroup namespace should immediately re-virtualize
 * `/proc/self/cgroup` against that namespace's root.
 */
FN_TEST(setns_revirtualizes_proc_cgroup)
{
	ino_t child_ns;
	ino_t joined_ns;
	pid_t child;
	int pidfd;

	TEST_SUCC(move_pid_to_cgroup(TEST_BASE_CGROUP, getpid()));
	TEST_SUCC(check_proc_cgroup(getpid(), "0::/cgns-base\n"));

	child = spawn_paused_process(TEST_BASE_CGROUP, MOVE_TO_NEW_CGROUP_NS);
	TEST_SUCC(read_process_cgroup_ns_inode(child, &child_ns));

	pidfd = TEST_SUCC(syscall(SYS_pidfd_open, child, 0));
	TEST_SUCC(setns(pidfd, CLONE_NEWCGROUP));

	TEST_RES(read_self_cgroup_ns_inode(&joined_ns), joined_ns == child_ns);
	TEST_SUCC(check_proc_cgroup(getpid(), "0::/\n"));

	TEST_SUCC(reset_self());
	TEST_SUCC(close(pidfd));
	kill_paused_process(child);
}
END_TEST()

/*
 * A fresh `cgroup2` mount inside a cgroup namespace starts at the namespace
 * root, so descendants below that root stay visible and siblings above it do
 * not appear in the mount tree.
 */
FN_TEST(cgroup_mount_starts_at_namespace_root)
{
	TEST_SUCC(move_pid_to_cgroup(TEST_BASE_CGROUP, getpid()));
	TEST_SUCC(unshare(CLONE_NEWCGROUP));

	TEST_SUCC(unshare(CLONE_NEWNS));
	TEST_SUCC(mount("none", TEST_MOUNT_DIR, "cgroup2", 0, NULL));

	TEST_SUCC(check_mount_root(TEST_MOUNT_DIR, "/"));
	TEST_SUCC(access(TEST_MOUNT_LEAF, F_OK));
	TEST_ERRNO(access(TEST_MOUNT_SIBLING, F_OK), ENOENT);

	TEST_SUCC(umount(TEST_MOUNT_DIR));
	TEST_SUCC(reset_self());
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(rmdir(TEST_MOUNT_DIR));
	CHECK(rmdir(TEST_LEAF_CGROUP));
	CHECK(rmdir(TEST_BASE_CGROUP));
	CHECK(rmdir(TEST_SIBLING_CGROUP));
	CHECK(close(g_initial_mount_ns_fd));
	CHECK(close(g_initial_cgroup_ns_fd));
}
END_SETUP()