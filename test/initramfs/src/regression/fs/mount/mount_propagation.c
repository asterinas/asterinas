// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <signal.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define STACK_SIZE (1024 * 1024)

#define WORK_DIR "/tmp/mount_propagation"

static int parent_to_child[2];
static int child_to_parent[2];

struct propagation_info {
	unsigned int shared;
	unsigned int master;
};

static int read_propagation_info(const char *target,
				 struct propagation_info *info)
{
	FILE *mountinfo = fopen("/proc/self/mountinfo", "r");
	if (!mountinfo)
		return -1;

	char line[4096];
	char mountpoint[4096];
	while (fgets(line, sizeof(line), mountinfo)) {
		char *separator = strstr(line, " - ");
		if (!separator)
			continue;
		*separator = '\0';

		if (sscanf(line, "%*u %*u %*s %*s %4095s", mountpoint) != 1 ||
		    strcmp(mountpoint, target) != 0)
			continue;

		info->shared = 0;
		info->master = 0;
		char *shared = strstr(line, " shared:");
		char *master = strstr(line, " master:");
		if (shared)
			sscanf(shared, " shared:%u", &info->shared);
		if (master)
			sscanf(master, " master:%u", &info->master);

		fclose(mountinfo);
		return 0;
	}

	fclose(mountinfo);
	return -1;
}

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

FN_SETUP(init)
{
	CHECK(mkdir(WORK_DIR, 0755));
}
END_SETUP()

static void notify_fd(int fd)
{
	char byte = 'X';
	CHECK_WITH(write(fd, &byte, 1), _ret == 1);
}

static void wait_fd(int fd)
{
	char byte;
	CHECK_WITH(read(fd, &byte, 1), _ret == 1 && byte == 'X');
}

static int isolate_mount_namespace(void)
{
	if (unshare(CLONE_NEWNS) < 0)
		return -1;
	return mount(NULL, "/", NULL, MS_REC | MS_PRIVATE, NULL);
}

static int propagation_child_fn(void *arg)
{
	(void)arg;
	wait_fd(parent_to_child[0]);

	CHECK(access(WORK_DIR "/master/from_parent", F_OK));

	/* A slave receives events from its master but cannot send events back. */
	CHECK(mount(NULL, WORK_DIR "/master", NULL, MS_SLAVE, NULL));
	ensure_dir(WORK_DIR "/master/from_child");
	CHECK(mount("child", WORK_DIR "/master/from_child", "tmpfs", 0, NULL));
	int fd = CHECK(open(WORK_DIR "/master/from_child/child_file",
			    O_CREAT | O_WRONLY, 0644));
	CHECK(close(fd));

	notify_fd(child_to_parent[1]);
	pause();
	return 0;
}

FN_TEST(shared_slave_propagation)
{
	ensure_dir(WORK_DIR "/master");

	TEST_SUCC(mount("master", WORK_DIR "/master", "tmpfs", 0, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/master", NULL, MS_SHARED, NULL));

	TEST_SUCC(pipe(parent_to_child));
	TEST_SUCC(pipe(child_to_parent));

	char *stack = malloc(STACK_SIZE);
	char *stack_top = stack + STACK_SIZE;
	pid_t pid = TEST_SUCC(clone(propagation_child_fn, stack_top,
				    CLONE_NEWNS | SIGCHLD, NULL));

	/* A mount added below the shared master reaches the child namespace. */
	ensure_dir(WORK_DIR "/master/from_parent");
	TEST_SUCC(mount("parent", WORK_DIR "/master/from_parent", "tmpfs", 0,
			NULL));

	notify_fd(parent_to_child[1]);
	wait_fd(child_to_parent[0]);

	TEST_ERRNO(access(WORK_DIR "/master/from_child/child_file", F_OK),
		   ENOENT);

	TEST_SUCC(kill(pid, SIGKILL));
	TEST_SUCC(waitpid(pid, NULL, 0));

	free(stack);
	TEST_SUCC(umount(WORK_DIR "/master/from_parent"));
	TEST_SUCC(rmdir(WORK_DIR "/master/from_parent"));
	TEST_SUCC(rmdir(WORK_DIR "/master/from_child"));
	TEST_SUCC(umount(WORK_DIR "/master"));
	TEST_SUCC(rmdir(WORK_DIR "/master"));
}
END_TEST()

FN_TEST(slave_one_way)
{
	TEST_SUCC(isolate_mount_namespace());

	ensure_dir(WORK_DIR "/master");
	ensure_dir(WORK_DIR "/slave");
	TEST_SUCC(mount("master", WORK_DIR "/master", "tmpfs", 0, NULL));
	ensure_dir(WORK_DIR "/master/event");
	TEST_SUCC(mount(NULL, WORK_DIR "/master", NULL, MS_SHARED, NULL));
	TEST_SUCC(mount(WORK_DIR "/master", WORK_DIR "/slave", NULL, MS_BIND,
			NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/slave", NULL, MS_SLAVE, NULL));

	TEST_SUCC(mount("event", WORK_DIR "/master/event", "tmpfs", 0, NULL));

	struct propagation_info master_info;
	struct propagation_info slave_info;
	TEST_RES(read_propagation_info(WORK_DIR "/master/event", &master_info),
		 _ret == 0 && master_info.shared > 0 &&
			 master_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/slave/event", &slave_info),
		 _ret == 0 && slave_info.shared == 0 &&
			 slave_info.master == master_info.shared);

	/* Events created under a slave do not propagate back to its master. */
	ensure_dir(WORK_DIR "/slave/event/local");
	TEST_SUCC(mount("local", WORK_DIR "/slave/event/local", "tmpfs", 0,
			NULL));
	int marker_fd = TEST_SUCC(open(WORK_DIR "/slave/event/local/marker",
				       O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(marker_fd));
	TEST_ERRNO(access(WORK_DIR "/master/event/local/marker", F_OK), ENOENT);

	TEST_SUCC(umount(WORK_DIR "/slave/event/local"));
	TEST_SUCC(umount(WORK_DIR "/master/event"));
	TEST_SUCC(umount(WORK_DIR "/slave"));
	TEST_SUCC(umount(WORK_DIR "/master"));
	TEST_SUCC(rmdir(WORK_DIR "/master"));
	TEST_SUCC(rmdir(WORK_DIR "/slave"));
}
END_TEST()

FN_TEST(shared_peer_group)
{
	TEST_SUCC(isolate_mount_namespace());

	ensure_dir(WORK_DIR "/source");
	ensure_dir(WORK_DIR "/first");
	ensure_dir(WORK_DIR "/second");
	TEST_SUCC(mount("source", WORK_DIR "/source", "tmpfs", 0, NULL));
	ensure_dir(WORK_DIR "/source/event");
	TEST_SUCC(mount(NULL, WORK_DIR "/source", NULL, MS_SHARED, NULL));
	/* Each bind mount becomes another receiver in the same peer group. */
	TEST_SUCC(mount(WORK_DIR "/source", WORK_DIR "/first", NULL, MS_BIND,
			NULL));
	TEST_SUCC(mount(WORK_DIR "/source", WORK_DIR "/second", NULL, MS_BIND,
			NULL));

	TEST_SUCC(mount("event", WORK_DIR "/source/event", "tmpfs", 0, NULL));

	struct propagation_info source_info;
	struct propagation_info first_info;
	struct propagation_info second_info;
	TEST_RES(read_propagation_info(WORK_DIR "/source/event", &source_info),
		 _ret == 0 && source_info.shared > 0 &&
			 source_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/first/event", &first_info),
		 _ret == 0 && first_info.shared == source_info.shared &&
			 first_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/second/event", &second_info),
		 _ret == 0 && second_info.shared == source_info.shared &&
			 second_info.master == 0);

	TEST_SUCC(umount(WORK_DIR "/source/event"));
	TEST_SUCC(umount(WORK_DIR "/second"));
	TEST_SUCC(umount(WORK_DIR "/first"));
	TEST_SUCC(umount(WORK_DIR "/source"));
	TEST_SUCC(rmdir(WORK_DIR "/source"));
	TEST_SUCC(rmdir(WORK_DIR "/first"));
	TEST_SUCC(rmdir(WORK_DIR "/second"));
}
END_TEST()

FN_TEST(move_into_shared)
{
	TEST_SUCC(isolate_mount_namespace());

	ensure_dir(WORK_DIR "/source");
	ensure_dir(WORK_DIR "/target");
	TEST_SUCC(mount("source", WORK_DIR "/source", "tmpfs", 0, NULL));
	TEST_SUCC(mount("target", WORK_DIR "/target", "tmpfs", 0, NULL));
	ensure_dir(WORK_DIR "/target/destination");
	TEST_SUCC(mount(NULL, WORK_DIR "/target", NULL, MS_SHARED, NULL));

	struct propagation_info moved_info;
	/* Moving a mount into a shared target enrolls it in that peer group. */
	TEST_SUCC(mount(WORK_DIR "/source", WORK_DIR "/target/destination",
			NULL, MS_MOVE, NULL));
	TEST_RES(read_propagation_info(WORK_DIR "/target/destination",
				       &moved_info),
		 _ret == 0 && moved_info.shared > 0 && moved_info.master == 0);

	TEST_SUCC(umount(WORK_DIR "/target/destination"));
	TEST_SUCC(umount(WORK_DIR "/target"));
	TEST_SUCC(rmdir(WORK_DIR "/source"));
	TEST_SUCC(rmdir(WORK_DIR "/target"));
}
END_TEST()

FN_TEST(move_shared_peer)
{
	TEST_SUCC(isolate_mount_namespace());

	ensure_dir(WORK_DIR "/source");
	ensure_dir(WORK_DIR "/target");
	ensure_dir(WORK_DIR "/peer");
	TEST_SUCC(mount("source", WORK_DIR "/source", "tmpfs", 0, NULL));
	int marker_fd = TEST_SUCC(
		open(WORK_DIR "/source/marker", O_CREAT | O_WRONLY, 0644));
	TEST_SUCC(close(marker_fd));
	TEST_SUCC(mount("target", WORK_DIR "/target", "tmpfs", 0, NULL));
	ensure_dir(WORK_DIR "/target/destination");
	TEST_SUCC(mount(NULL, WORK_DIR "/target", NULL, MS_SHARED, NULL));
	TEST_SUCC(mount(WORK_DIR "/target", WORK_DIR "/peer", NULL, MS_BIND,
			NULL));

	TEST_SUCC(mount(WORK_DIR "/source", WORK_DIR "/target/destination",
			NULL, MS_MOVE, NULL));
	/* Both peers receive the moved subtree and its marker. */
	TEST_SUCC(access(WORK_DIR "/target/destination/marker", F_OK));
	TEST_SUCC(access(WORK_DIR "/peer/destination/marker", F_OK));

	struct propagation_info moved_info;
	struct propagation_info copy_info;
	TEST_RES(read_propagation_info(WORK_DIR "/target/destination",
				       &moved_info),
		 _ret == 0 && moved_info.shared > 0);
	TEST_RES(read_propagation_info(WORK_DIR "/peer/destination",
				       &copy_info),
		 _ret == 0 && copy_info.shared == moved_info.shared);

	TEST_SUCC(umount(WORK_DIR "/target/destination"));
	TEST_ERRNO(access(WORK_DIR "/target/destination/marker", F_OK), ENOENT);
	TEST_ERRNO(access(WORK_DIR "/peer/destination/marker", F_OK), ENOENT);
	TEST_SUCC(umount(WORK_DIR "/peer"));
	TEST_SUCC(umount(WORK_DIR "/target"));
	TEST_SUCC(rmdir(WORK_DIR "/source"));
	TEST_SUCC(rmdir(WORK_DIR "/target"));
	TEST_SUCC(rmdir(WORK_DIR "/peer"));
}
END_TEST()

FN_TEST(move_unbindable_fails)
{
	TEST_SUCC(isolate_mount_namespace());

	ensure_dir(WORK_DIR "/source");
	ensure_dir(WORK_DIR "/target");
	TEST_SUCC(mount("source", WORK_DIR "/source", "tmpfs", 0, NULL));
	ensure_dir(WORK_DIR "/source/child");
	TEST_SUCC(mount("child", WORK_DIR "/source/child", "tmpfs", 0, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/source/child", NULL, MS_UNBINDABLE,
			NULL));
	TEST_SUCC(mount("target", WORK_DIR "/target", "tmpfs", 0, NULL));
	ensure_dir(WORK_DIR "/target/destination");
	TEST_SUCC(mount(NULL, WORK_DIR "/target", NULL, MS_SHARED, NULL));

	/* Shared mounts cannot accept a move containing an unbindable subtree. */
	TEST_ERRNO(mount(WORK_DIR "/source", WORK_DIR "/target/destination",
			 NULL, MS_MOVE, NULL),
		   EINVAL);

	TEST_SUCC(umount(WORK_DIR "/source/child"));
	TEST_SUCC(umount(WORK_DIR "/source"));
	TEST_SUCC(umount(WORK_DIR "/target"));
	TEST_SUCC(rmdir(WORK_DIR "/source"));
	TEST_SUCC(rmdir(WORK_DIR "/target"));
}
END_TEST()

FN_TEST(slave_private_after_unmount)
{
	ensure_dir(WORK_DIR "/master");
	ensure_dir(WORK_DIR "/slave");

	TEST_SUCC(mount("master", WORK_DIR "/master", "tmpfs", 0, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/master", NULL, MS_SHARED, NULL));
	TEST_SUCC(mount(WORK_DIR "/master", WORK_DIR "/slave", NULL, MS_BIND,
			NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/slave", NULL, MS_SLAVE, NULL));

	struct propagation_info master_info;
	struct propagation_info slave_info;
	TEST_RES(read_propagation_info(WORK_DIR "/master", &master_info),
		 _ret == 0 && master_info.shared > 0 &&
			 master_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/slave", &slave_info),
		 _ret == 0 && slave_info.shared == 0 &&
			 slave_info.master == master_info.shared);

	/* Removing the last master peer leaves the slave private and masterless. */
	TEST_SUCC(umount(WORK_DIR "/master"));
	TEST_RES(read_propagation_info(WORK_DIR "/slave", &slave_info),
		 _ret == 0 && slave_info.shared == 0 && slave_info.master == 0);

	TEST_SUCC(umount(WORK_DIR "/slave"));
	TEST_SUCC(rmdir(WORK_DIR "/master"));
	TEST_SUCC(rmdir(WORK_DIR "/slave"));
}
END_TEST()

FN_TEST(slave_reparented)
{
	ensure_dir(WORK_DIR "/upstream");
	ensure_dir(WORK_DIR "/master");
	ensure_dir(WORK_DIR "/slave");

	TEST_SUCC(mount("upstream", WORK_DIR "/upstream", "tmpfs", 0, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/upstream", NULL, MS_SHARED, NULL));
	TEST_SUCC(mount(WORK_DIR "/upstream", WORK_DIR "/master", NULL, MS_BIND,
			NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/master", NULL, MS_SLAVE, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/master", NULL, MS_SHARED, NULL));
	TEST_SUCC(mount(WORK_DIR "/master", WORK_DIR "/slave", NULL, MS_BIND,
			NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/slave", NULL, MS_SLAVE, NULL));

	struct propagation_info upstream_info;
	struct propagation_info master_info;
	struct propagation_info slave_info;
	TEST_RES(read_propagation_info(WORK_DIR "/upstream", &upstream_info),
		 _ret == 0 && upstream_info.shared > 0 &&
			 upstream_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/master", &master_info),
		 _ret == 0 && master_info.shared > 0 &&
			 master_info.shared != upstream_info.shared &&
			 master_info.master == upstream_info.shared);
	TEST_RES(read_propagation_info(WORK_DIR "/slave", &slave_info),
		 _ret == 0 && slave_info.shared == 0 &&
			 slave_info.master == master_info.shared);

	/* The remaining slave follows the upstream peer group after reparenting. */
	TEST_SUCC(umount(WORK_DIR "/master"));
	TEST_RES(read_propagation_info(WORK_DIR "/slave", &slave_info),
		 _ret == 0 && slave_info.shared == 0 &&
			 slave_info.master == upstream_info.shared);

	TEST_SUCC(umount(WORK_DIR "/slave"));
	TEST_SUCC(umount(WORK_DIR "/upstream"));
	TEST_SUCC(rmdir(WORK_DIR "/upstream"));
	TEST_SUCC(rmdir(WORK_DIR "/master"));
	TEST_SUCC(rmdir(WORK_DIR "/slave"));
}
END_TEST()

FN_TEST(singleton_slave_fallback)
{
	TEST_SUCC(isolate_mount_namespace());

	ensure_dir(WORK_DIR "/upstream");
	ensure_dir(WORK_DIR "/target");
	TEST_SUCC(mount("upstream", WORK_DIR "/upstream", "tmpfs", 0, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/upstream", NULL, MS_SHARED, NULL));
	TEST_SUCC(mount(WORK_DIR "/upstream", WORK_DIR "/target", NULL, MS_BIND,
			NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/target", NULL, MS_SLAVE, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/target", NULL, MS_SHARED, NULL));

	struct propagation_info upstream_info;
	struct propagation_info singleton_info;
	TEST_RES(read_propagation_info(WORK_DIR "/upstream", &upstream_info),
		 _ret == 0 && upstream_info.shared > 0 &&
			 upstream_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/target", &singleton_info),
		 _ret == 0 && singleton_info.shared > 0 &&
			 singleton_info.master == upstream_info.shared);

	/* Dropping shared propagation keeps the upstream master relationship. */
	TEST_SUCC(mount(NULL, WORK_DIR "/target", NULL, MS_SLAVE, NULL));
	TEST_RES(read_propagation_info(WORK_DIR "/target", &singleton_info),
		 _ret == 0 && singleton_info.shared == 0 &&
			 singleton_info.master == upstream_info.shared);

	TEST_SUCC(umount(WORK_DIR "/target"));
	TEST_SUCC(umount(WORK_DIR "/upstream"));
	TEST_SUCC(rmdir(WORK_DIR "/upstream"));
	TEST_SUCC(rmdir(WORK_DIR "/target"));
}
END_TEST()

FN_TEST(recursive_slave_no_reuse)
{
	TEST_SUCC(isolate_mount_namespace());

	ensure_dir(WORK_DIR "/parent");
	TEST_SUCC(mount("parent", WORK_DIR "/parent", "tmpfs", 0, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/parent", NULL, MS_SHARED, NULL));
	ensure_dir(WORK_DIR "/parent/child");
	TEST_SUCC(mount(WORK_DIR "/parent", WORK_DIR "/parent/child", NULL,
			MS_BIND, NULL));
	TEST_SUCC(mount(NULL, WORK_DIR "/parent/child", NULL, MS_SLAVE, NULL));

	struct propagation_info parent_info;
	struct propagation_info child_info;
	TEST_RES(read_propagation_info(WORK_DIR "/parent", &parent_info),
		 _ret == 0 && parent_info.shared > 0 &&
			 parent_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/parent/child", &child_info),
		 _ret == 0 && child_info.shared == 0 &&
			 child_info.master == parent_info.shared);

	/* Recursive slave conversion must not restore the old peer group. */
	TEST_SUCC(
		mount(NULL, WORK_DIR "/parent", NULL, MS_REC | MS_SLAVE, NULL));
	TEST_RES(read_propagation_info(WORK_DIR "/parent", &parent_info),
		 _ret == 0 && parent_info.shared == 0 &&
			 parent_info.master == 0);
	TEST_RES(read_propagation_info(WORK_DIR "/parent/child", &child_info),
		 _ret == 0 && child_info.shared == 0 && child_info.master == 0);

	TEST_SUCC(umount(WORK_DIR "/parent/child"));
	TEST_SUCC(umount(WORK_DIR "/parent"));
	TEST_SUCC(rmdir(WORK_DIR "/parent"));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(rmdir(WORK_DIR));
}
END_SETUP()
