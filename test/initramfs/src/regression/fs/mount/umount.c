// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/mount.h>
#include <linux/stat.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdatomic.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#define UNREACHABLE_PREFIX "(unreachable)"
#define GETCWD_MOUNT_RACE_ROUNDS 500

#define BUSY_PARENT "/tmp/umount_busy_a"
#define BUSY_CHILD "/tmp/umount_busy_a/b"
#define BUSY_DEEP "/tmp/umount_busy_a/b/deep"

#define DETACH_PARENT "/tmp/umount_detach_a"
#define DETACH_CHILD "/tmp/umount_detach_a/b"
#define DETACH_DEEP "/tmp/umount_detach_a/b/deep"

#define RACE_PARENT "/tmp/umount_race_a"
#define RACE_CHILD "/tmp/umount_race_a/b"
#define RACE_DEEP "/tmp/umount_race_a/b/deep"
#define RACE_SIBLING "/tmp/umount_race_a/b/sibling"

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret == 0 || errno == EEXIST);
}

static void mount_tmpfs(const char *path)
{
	ensure_dir(path);
	CHECK(mount("none", path, "tmpfs", 0, NULL));
}

static uint64_t mount_unique_id(const char *path)
{
	struct statx stx;
	int r = statx(AT_FDCWD, path, 0, STATX_MNT_ID_UNIQUE, &stx);
	if (r < 0) {
		return 0;
	}
	return stx.stx_mnt_id;
}

FN_TEST(regular_umount_of_parent_with_child_returns_ebusy)
{
	TEST_SUCC(unshare(CLONE_NEWNS));

	mount_tmpfs(BUSY_PARENT);
	mount_tmpfs(BUSY_CHILD);
	ensure_dir(BUSY_DEEP);
	TEST_SUCC(chdir(BUSY_DEEP));

	char cwd[PATH_MAX];
	TEST_RES(getcwd(cwd, sizeof(cwd)), strcmp(cwd, BUSY_DEEP) == 0);

	TEST_ERRNO(umount2(BUSY_PARENT, 0), EBUSY);
	TEST_ERRNO(umount2(BUSY_PARENT, MNT_EXPIRE | MNT_DETACH), EINVAL);
	TEST_ERRNO(umount2(BUSY_PARENT, MNT_EXPIRE | MNT_FORCE), EINVAL);
	TEST_RES(getcwd(cwd, sizeof(cwd)), strcmp(cwd, BUSY_DEEP) == 0);

	TEST_SUCC(chdir("/"));
	TEST_SUCC(umount(BUSY_CHILD));
	TEST_SUCC(umount(BUSY_PARENT));
	TEST_SUCC(rmdir(BUSY_PARENT));
}
END_TEST()

FN_TEST(mnt_detach_parent_leaves_getcwd_unreachable)
{
	TEST_SUCC(unshare(CLONE_NEWNS));

	mount_tmpfs(DETACH_PARENT);
	mount_tmpfs(DETACH_CHILD);
	ensure_dir(DETACH_DEEP);

	uint64_t parent_uid =
		TEST_RES(mount_unique_id(DETACH_PARENT), _ret != 0);
	uint64_t child_uid = TEST_RES(mount_unique_id(DETACH_CHILD), _ret != 0);

	TEST_SUCC(chdir(DETACH_DEEP));
	TEST_SUCC(umount2(DETACH_PARENT, MNT_DETACH));

	char cwd[PATH_MAX];
	TEST_RES(syscall(SYS_getcwd, cwd, sizeof(cwd)),
		 strncmp(cwd, UNREACHABLE_PREFIX, strlen(UNREACHABLE_PREFIX)) ==
			 0);

	struct mnt_id_req req = {
		.size = MNT_ID_REQ_SIZE_VER0,
		.mnt_id = parent_uid,
	};
	uint64_t ids[1];
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), ENOENT);
	req.mnt_id = child_uid;
	TEST_ERRNO(syscall(SYS_listmount, &req, ids, 1, 0), ENOENT);

	TEST_SUCC(chdir("/"));
	TEST_SUCC(rmdir(DETACH_PARENT));
}
END_TEST()

struct getcwd_race {
	const char *expected;
	atomic_int stop;
	atomic_int failed;
	atomic_ulong iterations;
};

static void *getcwd_racer(void *arg)
{
	struct getcwd_race *race = arg;
	char buf[PATH_MAX];

	while (!atomic_load_explicit(&race->stop, memory_order_acquire)) {
		long n = syscall(SYS_getcwd, buf, sizeof(buf));
		if (n <= 0 || strcmp(buf, race->expected) != 0) {
			atomic_store_explicit(&race->failed, 1,
					      memory_order_release);
			break;
		}
		atomic_fetch_add_explicit(&race->iterations, 1,
					  memory_order_relaxed);
	}

	return NULL;
}

FN_TEST(getcwd_is_stable_across_concurrent_mount_unmount)
{
	TEST_SUCC(unshare(CLONE_NEWNS));

	mount_tmpfs(RACE_PARENT);
	mount_tmpfs(RACE_CHILD);
	ensure_dir(RACE_DEEP);
	ensure_dir(RACE_SIBLING);
	TEST_SUCC(chdir(RACE_DEEP));

	struct getcwd_race race = {
		.expected = RACE_DEEP,
		.stop = 0,
		.failed = 0,
		.iterations = 0,
	};
	pthread_t thread;
	TEST_SUCC(pthread_create(&thread, NULL, getcwd_racer, &race));
	while (atomic_load_explicit(&race.iterations, memory_order_relaxed) ==
		       0 &&
	       !atomic_load_explicit(&race.failed, memory_order_acquire)) {
		sched_yield();
	}
	TEST_RES(atomic_load_explicit(&race.failed, memory_order_acquire),
		 _ret == 0);
	unsigned long iterations_before =
		atomic_load_explicit(&race.iterations, memory_order_relaxed);

	int parent_busy_ok = 1;
	int sibling_ok = 1;
	for (int i = 0; i < GETCWD_MOUNT_RACE_ROUNDS; i++) {
		if (umount2(RACE_PARENT, 0) != -1 || errno != EBUSY) {
			parent_busy_ok = 0;
			break;
		}
		if (mount("none", RACE_SIBLING, "tmpfs", 0, NULL) < 0 ||
		    umount(RACE_SIBLING) < 0) {
			sibling_ok = 0;
			break;
		}
		sched_yield();
	}

	unsigned long iterations_after =
		atomic_load_explicit(&race.iterations, memory_order_relaxed);
	atomic_store_explicit(&race.stop, 1, memory_order_release);
	TEST_SUCC(pthread_join(thread, NULL));
	TEST_RES(parent_busy_ok, _ret == 1);
	TEST_RES(sibling_ok, _ret == 1);
	TEST_RES(atomic_load_explicit(&race.failed, memory_order_acquire),
		 _ret == 0);
	TEST_RES(iterations_after, _ret > iterations_before);

	TEST_SUCC(chdir("/"));
	TEST_SUCC(umount(RACE_CHILD));
	TEST_SUCC(umount(RACE_PARENT));
	TEST_SUCC(rmdir(RACE_PARENT));
}
END_TEST()
